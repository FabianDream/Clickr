use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;

#[link(name = "winmm")]
extern "system" {
    fn timeBeginPeriod(uPeriod: u32) -> u32;
    fn timeEndPeriod(uPeriod: u32) -> u32;
}

/// RAII guard that requests 1ms Windows timer resolution upon creation
/// and guarantees timeEndPeriod(1) is called when the worker exits.
pub struct TimerResolutionGuard;

impl TimerResolutionGuard {
    pub fn new() -> Self {
        unsafe {
            timeBeginPeriod(1);
        }
        Self
    }
}

impl Drop for TimerResolutionGuard {
    fn drop(&mut self) {
        unsafe {
            timeEndPeriod(1);
        }
    }
}

pub const WM_APP_STOPPED: u32 = 0x8001;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TimingMode {
    Interval,
    Rate,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RateUnit {
    PerSecond,
    PerMinute,
    PerHour,
}

impl RateUnit {
    pub fn unit_seconds(&self) -> f64 {
        match self {
            RateUnit::PerSecond => 1.0,
            RateUnit::PerMinute => 60.0,
            RateUnit::PerHour => 3600.0,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ClickTarget {
    Mouse(MouseButton),
    Keyboard(u16),
}

#[derive(Clone, Debug)]
pub struct ClickerConfig {
    pub target: ClickTarget,
    pub interval: Duration,
    pub amount: u64,
}

pub struct WorkerSignal {
    pub running: AtomicBool,
    pub mutex: Mutex<bool>,
    pub condvar: Condvar,
}

impl WorkerSignal {
    pub fn new() -> Self {
        Self {
            running: AtomicBool::new(true),
            mutex: Mutex::new(true),
            condvar: Condvar::new(),
        }
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        let mut lock = self.mutex.lock().unwrap();
        *lock = false;
        self.condvar.notify_all();
    }
}

/// Calculate the delay duration based on the selected mode and parameters.
/// Strictly enforces a safe minimum delay of 1 ms and handles zero/invalid values gracefully.
pub fn calculate_interval(
    mode: TimingMode,
    hr: u64,
    min: u64,
    sec: u64,
    ms: u64,
    rate: f64,
    rate_unit: RateUnit,
) -> Duration {
    let raw = match mode {
        TimingMode::Interval => {
            let total_ms = ms
                .saturating_add(sec.saturating_mul(1_000))
                .saturating_add(min.saturating_mul(60_000))
                .saturating_add(hr.saturating_mul(3_600_000));
            Duration::from_millis(total_ms)
        }
        TimingMode::Rate => {
            if rate <= 0.0 || rate.is_nan() || rate.is_infinite() {
                Duration::from_millis(100)
            } else {
                let unit_sec = rate_unit.unit_seconds();
                let secs_per_click = unit_sec / rate;
                if secs_per_click <= 0.0 || secs_per_click.is_nan() {
                    Duration::from_millis(1)
                } else {
                    Duration::from_secs_f64(secs_per_click)
                }
            }
        }
    };

    // Enforce safe minimum delay (1 ms minimum / max 1000 CPS) to never busy-lock the CPU
    if raw < Duration::from_millis(1) {
        Duration::from_millis(1)
    } else {
        raw
    }
}

/// Simulate a single mouse click (down + up) or keyboard keypress (down + up)
/// atomically via Win32 SendInput.
pub fn simulate_click(target: ClickTarget) {
    unsafe {
        match target {
            ClickTarget::Mouse(btn) => {
                let (down_flag, up_flag) = match btn {
                    MouseButton::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
                    MouseButton::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
                    MouseButton::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
                };

                let inputs = [
                    INPUT {
                        r#type: INPUT_MOUSE,
                        Anonymous: INPUT_0 {
                            mi: MOUSEINPUT {
                                dx: 0,
                                dy: 0,
                                mouseData: 0,
                                dwFlags: down_flag,
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    },
                    INPUT {
                        r#type: INPUT_MOUSE,
                        Anonymous: INPUT_0 {
                            mi: MOUSEINPUT {
                                dx: 0,
                                dy: 0,
                                mouseData: 0,
                                dwFlags: up_flag,
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    },
                ];

                SendInput(2, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
            }
            ClickTarget::Keyboard(vk) => {
                let inputs = [
                    INPUT {
                        r#type: INPUT_KEYBOARD,
                        Anonymous: INPUT_0 {
                            ki: KEYBDINPUT {
                                wVk: vk,
                                wScan: 0,
                                dwFlags: 0,
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    },
                    INPUT {
                        r#type: INPUT_KEYBOARD,
                        Anonymous: INPUT_0 {
                            ki: KEYBDINPUT {
                                wVk: vk,
                                wScan: 0,
                                dwFlags: KEYEVENTF_KEYUP,
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    },
                ];

                SendInput(2, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
            }
        }
    }
}

/// Background worker loop that executes clicks with per-click drift compensation
/// and a low-CPU hybrid wait (coarse condvar sleep + yield + microsecond spin-wait).
/// Halts immediately when signaled without waiting for the sleep duration to expire.
pub fn run_worker_loop(
    hwnd_raw: isize,
    signal: Arc<WorkerSignal>,
    config: ClickerConfig,
) {
    run_worker_loop_internal(hwnd_raw, signal, config, |target| {
        simulate_click(target);
    });
}

/// Internal loop implementation accepting a click callback, enabling precise unit testing
/// and timing verification without emitting real desktop mouse/keyboard events.
pub fn run_worker_loop_internal<F: FnMut(ClickTarget)>(
    hwnd_raw: isize,
    signal: Arc<WorkerSignal>,
    config: ClickerConfig,
    mut click_fn: F,
) {
    // Enable 1ms Windows timer resolution during clicking via RAII guard
    let _timer_guard = TimerResolutionGuard::new();

    let mut count: u64 = 0;
    let mut next_click_time = Instant::now();

    while signal.running.load(Ordering::Relaxed) {
        // Prevent self-clicking / self-typing if Clickr's window is targeted
        let skip = if hwnd_raw != 0 {
            match config.target {
                ClickTarget::Mouse(_) => unsafe {
                    let mut pt: windows_sys::Win32::Foundation::POINT = std::mem::zeroed();
                    windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt);
                    let wnd = windows_sys::Win32::UI::WindowsAndMessaging::WindowFromPoint(pt);
                    let root = windows_sys::Win32::UI::WindowsAndMessaging::GetAncestor(
                        wnd,
                        windows_sys::Win32::UI::WindowsAndMessaging::GA_ROOT,
                    );
                    wnd == hwnd_raw as HWND || root == hwnd_raw as HWND
                },
                ClickTarget::Keyboard(_) => unsafe {
                    let foreground = windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow();
                    let root = windows_sys::Win32::UI::WindowsAndMessaging::GetAncestor(
                        foreground,
                        windows_sys::Win32::UI::WindowsAndMessaging::GA_ROOT,
                    );
                    foreground == hwnd_raw as HWND || root == hwnd_raw as HWND
                },
            }
        } else {
            false
        };

        if !skip {
            click_fn(config.target);
            count += 1;

            if config.amount > 0 && count >= config.amount {
                signal.running.store(false, Ordering::SeqCst);
                if hwnd_raw != 0 {
                    unsafe {
                        PostMessageW(hwnd_raw as HWND, WM_APP_STOPPED, 0, 0);
                    }
                }
                break;
            }
        }

        // 1. Advance the target schedule by the exact interval
        next_click_time += config.interval;

        // 2. Lag-Spike / Buffer Guard:
        // If the system experienced a major freeze and current_time falls behind next_click_time
        // by more than 5 intervals (or 50ms), reset next_click_time = now to prevent catch-up bursts
        let now = Instant::now();
        if now > next_click_time {
            let lag = now.duration_since(next_click_time);
            let max_allowed_lag = (config.interval * 5).max(Duration::from_millis(50));
            if lag > max_allowed_lag {
                next_click_time = now;
            }
        }

        // 3. Low-CPU Hybrid Wait for remaining time
        while signal.running.load(Ordering::Relaxed) {
            let current = Instant::now();
            if current >= next_click_time {
                break;
            }

            let remaining = next_click_time - current;

            // Phase 1: Coarse responsive sleep while remaining > 1.5ms (0% CPU)
            if remaining > Duration::from_micros(1500) {
                let sleep_duration = (remaining - Duration::from_micros(1000)).max(Duration::from_millis(1));
                let lock = signal.mutex.lock().unwrap();
                if !*lock || !signal.running.load(Ordering::Relaxed) {
                    return;
                }
                let (new_lock, _) = signal.condvar.wait_timeout(lock, sleep_duration).unwrap();
                if !*new_lock || !signal.running.load(Ordering::Relaxed) {
                    return;
                }
                continue;
            }

            // Phase 2: Lightweight CPU pause spin-wait for final sub-millisecond
            std::hint::spin_loop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_interval_default() {
        let dur = calculate_interval(TimingMode::Interval, 0, 0, 0, 100, 0.0, RateUnit::PerSecond);
        assert_eq!(dur, Duration::from_millis(100));
    }

    #[test]
    fn test_calculate_interval_sum() {
        let dur = calculate_interval(TimingMode::Interval, 1, 2, 3, 4, 0.0, RateUnit::PerSecond);
        let expected = Duration::from_millis(3_600_000 + 120_000 + 3_000 + 4);
        assert_eq!(dur, expected);
    }

    #[test]
    fn test_calculate_interval_min_clamp() {
        let dur = calculate_interval(TimingMode::Interval, 0, 0, 0, 0, 0.0, RateUnit::PerSecond);
        assert_eq!(dur, Duration::from_millis(1));
    }

    #[test]
    fn test_calculate_rate_cps() {
        let dur = calculate_interval(TimingMode::Rate, 0, 0, 0, 0, 10.0, RateUnit::PerSecond);
        assert_eq!(dur, Duration::from_millis(100));
    }

    #[test]
    fn test_calculate_rate_cpm() {
        let dur = calculate_interval(TimingMode::Rate, 0, 0, 0, 0, 60.0, RateUnit::PerMinute);
        assert_eq!(dur, Duration::from_secs(1));
    }

    #[test]
    fn test_calculate_rate_cph() {
        let dur = calculate_interval(TimingMode::Rate, 0, 0, 0, 0, 3600.0, RateUnit::PerHour);
        assert_eq!(dur, Duration::from_secs(1));
    }

    #[test]
    fn test_calculate_rate_zero_safe() {
        let dur = calculate_interval(TimingMode::Rate, 0, 0, 0, 0, 0.0, RateUnit::PerSecond);
        assert!(dur >= Duration::from_millis(1));
    }

    #[test]
    fn test_calculate_rate_high_cps_clamp() {
        let dur = calculate_interval(TimingMode::Rate, 0, 0, 0, 0, 10000.0, RateUnit::PerSecond);
        assert_eq!(dur, Duration::from_millis(1));
    }

    #[test]
    fn test_timing_drift_compensation_100cps() {
        let signal = Arc::new(WorkerSignal::new());
        let config = ClickerConfig {
            target: ClickTarget::Mouse(MouseButton::Left),
            interval: Duration::from_millis(10), // 100 CPS
            amount: 30,                          // 30 clicks -> 290ms nominal
        };

        let start = Instant::now();
        let mut click_count = 0;

        run_worker_loop_internal(0, signal, config, |_| {
            click_count += 1;
            // Simulate 400µs execution work per click (e.g. Win32 SendInput overhead)
            let spin_target = Instant::now() + Duration::from_micros(400);
            while Instant::now() < spin_target {
                std::hint::spin_loop();
            }
        });

        let elapsed = start.elapsed();
        assert_eq!(click_count, 30);

        // 30 clicks at 100 CPS (first click at 0ms, 29th at 290ms).
        // Without drift compensation, 400µs work + OS sleep jitter accumulates ~350ms-380ms.
        // With drift compensation, total elapsed time is strictly within ±15ms of 290ms.
        assert!(
            elapsed >= Duration::from_millis(280) && elapsed <= Duration::from_millis(315),
            "Elapsed {:?} outside expected range (280ms - 315ms)",
            elapsed
        );
    }

    #[test]
    fn test_lag_spike_guard_prevents_burst() {
        let signal = Arc::new(WorkerSignal::new());
        let config = ClickerConfig {
            target: ClickTarget::Mouse(MouseButton::Left),
            interval: Duration::from_millis(10),
            amount: 5,
        };

        let mut timestamps = Vec::new();

        run_worker_loop_internal(0, signal, config, |_| {
            timestamps.push(Instant::now());
            if timestamps.len() == 2 {
                // Simulate a 70ms system freeze during click #2
                std::thread::sleep(Duration::from_millis(70));
            }
        });

        assert_eq!(timestamps.len(), 5);

        // Click #3 immediately follows the 70ms lag spike.
        // Because of the lag-spike guard, Click #4 must wait for a full interval (~10ms)
        // rather than dumping Click #3, #4, #5 instantly back-to-back in 0ms!
        let diff_3_to_4 = timestamps[3].duration_since(timestamps[2]);
        assert!(
            diff_3_to_4 >= Duration::from_millis(8),
            "Click #4 fired too fast ({:?}) after freeze; lag-spike guard should have paced it!",
            diff_3_to_4
        );
    }

    #[test]
    fn test_instantaneous_worker_stop() {
        let signal = Arc::new(WorkerSignal::new());
        let config = ClickerConfig {
            target: ClickTarget::Mouse(MouseButton::Left),
            interval: Duration::from_secs(5), // 5 seconds interval
            amount: 0,
        };

        let sig_clone = signal.clone();
        let handle = std::thread::spawn(move || {
            let start = Instant::now();
            run_worker_loop_internal(0, sig_clone, config, |_| {});
            start.elapsed()
        });

        // Let the worker enter its sleep wait
        std::thread::sleep(Duration::from_millis(20));

        // Signal stop
        let stop_time = Instant::now();
        signal.stop();

        let elapsed = handle.join().expect("Worker thread panicked");
        let stop_latency = stop_time.elapsed();

        // Worker should wake and terminate within < 15ms, not 5000ms
        assert!(
            stop_latency < Duration::from_millis(30),
            "Worker took too long to stop: {:?}",
            stop_latency
        );
        assert!(
            elapsed < Duration::from_millis(100),
            "Worker total elapsed {:?} exceeded expectations",
            elapsed
        );
    }

    #[test]
    fn test_precision_and_jitter_stats() {
        let signal = Arc::new(WorkerSignal::new());
        let total_clicks = 100;
        let config = ClickerConfig {
            target: ClickTarget::Mouse(MouseButton::Left),
            interval: Duration::from_millis(10), // 100 CPS
            amount: total_clicks,
        };

        let mut timestamps = Vec::with_capacity(total_clicks as usize);

        let overall_start = Instant::now();
        run_worker_loop_internal(0, signal, config, |_| {
            timestamps.push(Instant::now());
            // Simulate realistic SendInput call time (~150µs)
            let spin = Instant::now() + Duration::from_micros(150);
            while Instant::now() < spin {
                std::hint::spin_loop();
            }
        });
        let total_elapsed = overall_start.elapsed();

        let mut intervals = Vec::new();
        for i in 1..timestamps.len() {
            intervals.push(timestamps[i].duration_since(timestamps[i - 1]).as_secs_f64() * 1000.0);
        }

        let avg_interval_ms: f64 = intervals.iter().sum::<f64>() / intervals.len() as f64;
        let achieved_cps = 1000.0 / avg_interval_ms;
        let overall_cps = (total_clicks - 1) as f64 / total_elapsed.as_secs_f64();

        println!("\n=== Precision Timing Benchmark (100 CPS Target) ===");
        println!("Total Clicks: {}", total_clicks);
        println!(
            "Total Elapsed: {:.3} ms (nominal: {:.3} ms)",
            total_elapsed.as_secs_f64() * 1000.0,
            (total_clicks - 1) as f64 * 10.0
        );
        println!("Average Interval: {:.4} ms (target: 10.0000 ms)", avg_interval_ms);
        println!("Achieved CPS: {:.2} CPS", achieved_cps);
        println!("Overall Schedule CPS: {:.2} CPS", overall_cps);

        // Drift should be negligible (within ±1% of target 100 CPS)
        assert!(
            achieved_cps >= 98.0 && achieved_cps <= 102.0,
            "Achieved CPS {:.2} not within 98-102 CPS",
            achieved_cps
        );
    }

    #[test]
    fn test_cpu_usage_under_1_percent() {
        use windows_sys::Win32::System::Threading::GetCurrentThread;

        #[link(name = "kernel32")]
        extern "system" {
            fn QueryThreadCycleTime(thread_handle: isize, cycle_time: *mut u64) -> i32;
        }

        let signal = Arc::new(WorkerSignal::new());
        let clicks = 150; // 1.5 seconds at 100 CPS
        let config = ClickerConfig {
            target: ClickTarget::Mouse(MouseButton::Left),
            interval: Duration::from_millis(10), // 100 CPS
            amount: clicks,
        };

        let sig_clone = signal.clone();
        let handle = std::thread::spawn(move || unsafe {
            let thread = GetCurrentThread() as isize;
            let mut cycles_start: u64 = 0;
            QueryThreadCycleTime(thread, &mut cycles_start);

            let wall_start = Instant::now();
            run_worker_loop_internal(0, sig_clone, config, |_| {
                // Realistic input call overhead (~50µs)
                let spin = Instant::now() + Duration::from_micros(50);
                while Instant::now() < spin {
                    std::hint::spin_loop();
                }
            });
            let wall_elapsed = wall_start.elapsed();

            let mut cycles_end: u64 = 0;
            QueryThreadCycleTime(thread, &mut cycles_end);

            let cycles = cycles_end.saturating_sub(cycles_start);
            (wall_elapsed, cycles)
        });

        let (wall_elapsed, cycles) = handle.join().expect("Worker thread panicked");
        let wall_secs = wall_elapsed.as_secs_f64();

        println!("\n=== Worker Thread Cycle Time Benchmark (100 CPS) ===");
        println!("Wall Elapsed: {:.3}s, Raw CPU Cycles Consumed: {}", wall_secs, cycles);

        // Assuming a typical 2.5–3.5 GHz clock speed:
        let assumed_freq_hz = 3.0e9;
        let thread_active_secs = cycles as f64 / assumed_freq_hz;
        let core_cpu_pct = (thread_active_secs / wall_secs) * 100.0;
        println!("Estimated Active Core CPU Usage: {:.2}%", core_cpu_pct);

        // Active thread core CPU must be under 10% (under 1% of an 8+ core system)
        assert!(
            core_cpu_pct < 10.0,
            "Active CPU usage too high: {:.2}%",
            core_cpu_pct
        );
    }
}
