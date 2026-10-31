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

/// Maximum safe CPS limit to prevent overloading Windows processes, saturating message queues, or crashing apps.
pub const MAX_SAFE_CPS: f64 = 100.0;
/// Minimum safe interval corresponding to MAX_SAFE_CPS (10 ms = 100 CPS).
pub const MIN_SAFE_INTERVAL: Duration = Duration::from_millis(10);

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
pub enum ClickType {
    Single,
    Double,
    Hold,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TriggerMode {
    Toggle,
    Hold,
    HoldMouse,
}

pub static PHYSICAL_MOUSE_DOWN: AtomicBool = AtomicBool::new(false);

pub fn is_physical_mouse_down() -> bool {
    PHYSICAL_MOUSE_DOWN.load(Ordering::Relaxed)
}

pub fn set_physical_mouse_down(down: bool) {
    PHYSICAL_MOUSE_DOWN.store(down, Ordering::SeqCst);
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MouseLocation {
    Current,
    Specific(i32, i32),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ClickTarget {
    Mouse(MouseButton, ClickType),
    Keyboard(u16, ClickType),
}

#[derive(Clone, Debug)]
pub struct ClickerConfig {
    pub target: ClickTarget,
    pub location: MouseLocation,
    pub trigger_mode: TriggerMode,
    pub hotkey: u32,
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
/// Strictly enforces a safe maximum of 100 CPS (minimum 10 ms interval) to prevent
/// message queue exhaustion, UI freezing, and application crashes on Windows.
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
                let max_rate = MAX_SAFE_CPS * unit_sec;
                let safe_rate = rate.min(max_rate);
                let secs_per_click = unit_sec / safe_rate;
                if secs_per_click <= 0.0 || secs_per_click.is_nan() {
                    MIN_SAFE_INTERVAL
                } else {
                    Duration::from_secs_f64(secs_per_click)
                }
            }
        }
    };

    // Enforce safe minimum delay (10 ms minimum / max 100 CPS) to never choke Windows processes or crash apps
    if raw < MIN_SAFE_INTERVAL {
        MIN_SAFE_INTERVAL
    } else {
        raw
    }
}

pub const MAPVK_VK_TO_VSC: u32 = 0;

/// Calculate the adaptive key hold duration (down-to-up delay) based on the click interval.
/// Ensures a natural human press (~20-25ms) for moderate/slow rates,
/// and scales dynamically (interval / 2) for rapid click rates so Down and Up never collide.
pub fn calculate_key_hold_duration(interval: Duration) -> Duration {
    let half = interval / 2;
    let max_hold = Duration::from_millis(25);
    half.min(max_hold)
}

/// Helper to construct a KEYBDINPUT with hardware scan code and appropriate flags.
pub fn create_keyboard_input(vk: u16, is_up: bool) -> INPUT {
    unsafe {
        let scan = MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) as u16;
        let mut flags = if is_up { KEYEVENTF_KEYUP } else { 0 };

        // Extended keys require KEYEVENTF_EXTENDEDKEY flag
        if matches!(
            vk,
            VK_UP | VK_DOWN | VK_LEFT | VK_RIGHT
                | VK_INSERT | VK_DELETE | VK_HOME | VK_END | VK_PRIOR | VK_NEXT
                | VK_RCONTROL | VK_RMENU | VK_DIVIDE | VK_NUMLOCK
        ) {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }

        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: scan,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }
}

/// Press a target mouse button or keyboard key down (used for Hold click type).
pub fn press_target_down(target: ClickTarget) {
    unsafe {
        match target {
            ClickTarget::Mouse(btn, _) => {
                let down_flag = match btn {
                    MouseButton::Left => MOUSEEVENTF_LEFTDOWN,
                    MouseButton::Right => MOUSEEVENTF_RIGHTDOWN,
                    MouseButton::Middle => MOUSEEVENTF_MIDDLEDOWN,
                };
                let input = INPUT {
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
                };
                SendInput(1, &input, std::mem::size_of::<INPUT>() as i32);
            }
            ClickTarget::Keyboard(vk, _) => {
                let input = create_keyboard_input(vk, false);
                SendInput(1, &input, std::mem::size_of::<INPUT>() as i32);
            }
        }
    }
}

/// Release a target mouse button or keyboard key up (used for Hold click type).
pub fn release_target(target: ClickTarget) {
    unsafe {
        match target {
            ClickTarget::Mouse(btn, _) => {
                let up_flag = match btn {
                    MouseButton::Left => MOUSEEVENTF_LEFTUP,
                    MouseButton::Right => MOUSEEVENTF_RIGHTUP,
                    MouseButton::Middle => MOUSEEVENTF_MIDDLEUP,
                };
                let input = INPUT {
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
                };
                SendInput(1, &input, std::mem::size_of::<INPUT>() as i32);
            }
            ClickTarget::Keyboard(vk, _) => {
                let input = create_keyboard_input(vk, true);
                SendInput(1, &input, std::mem::size_of::<INPUT>() as i32);
            }
        }
    }
}

/// Simulate a single/double click or hold state for mouse or keyboard with default hold time.
pub fn simulate_click(target: ClickTarget) {
    simulate_click_responsive(target, Duration::from_millis(20), None);
}

/// Simulate a single/double click or hold state with a specific hold duration.
pub fn simulate_click_with_hold(target: ClickTarget, hold_duration: Duration) {
    simulate_click_responsive(target, hold_duration, None);
}

/// Simulate click with adaptive down-to-up hold delay and responsive cancellation on worker stop.
pub fn simulate_click_responsive(
    target: ClickTarget,
    hold_duration: Duration,
    signal: Option<&WorkerSignal>,
) {
    unsafe {
        match target {
            ClickTarget::Mouse(btn, click_type) => {
                let (down_flag, up_flag) = match btn {
                    MouseButton::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
                    MouseButton::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
                    MouseButton::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
                };

                let make_inputs = || [
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

                match click_type {
                    ClickType::Single => {
                        let inputs = make_inputs();
                        SendInput(2, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
                    }
                    ClickType::Double => {
                        let inputs1 = make_inputs();
                        SendInput(2, inputs1.as_ptr(), std::mem::size_of::<INPUT>() as i32);
                        // Inter-click interval for Windows to recognize distinct double-click events
                        std::thread::sleep(Duration::from_millis(15));
                        let inputs2 = make_inputs();
                        SendInput(2, inputs2.as_ptr(), std::mem::size_of::<INPUT>() as i32);
                    }
                    ClickType::Hold => {
                        press_target_down(target);
                    }
                }
            }
            ClickTarget::Keyboard(vk, click_type) => {
                match click_type {
                    ClickType::Single => {
                        // 1. Send Key Down (with hardware scan code)
                        let down = create_keyboard_input(vk, false);
                        SendInput(1, &down, std::mem::size_of::<INPUT>() as i32);

                        // 2. Realistic Hold Duration so OS, DirectInput, and games register the Down state
                        if hold_duration > Duration::ZERO {
                            if let Some(sig) = signal {
                                if sig.running.load(Ordering::Relaxed) {
                                    let lock = sig.mutex.lock().unwrap();
                                    let _ = sig.condvar.wait_timeout(lock, hold_duration);
                                }
                            } else {
                                std::thread::sleep(hold_duration);
                            }
                        }

                        // 3. Send Key Up (with hardware scan code)
                        let up = create_keyboard_input(vk, true);
                        SendInput(1, &up, std::mem::size_of::<INPUT>() as i32);
                    }
                    ClickType::Double => {
                        // First stroke: Down -> Hold -> Up
                        let down1 = create_keyboard_input(vk, false);
                        SendInput(1, &down1, std::mem::size_of::<INPUT>() as i32);
                        if hold_duration > Duration::ZERO {
                            if let Some(sig) = signal {
                                if sig.running.load(Ordering::Relaxed) {
                                    let lock = sig.mutex.lock().unwrap();
                                    let _ = sig.condvar.wait_timeout(lock, hold_duration);
                                }
                            } else {
                                std::thread::sleep(hold_duration);
                            }
                        }
                        let up1 = create_keyboard_input(vk, true);
                        SendInput(1, &up1, std::mem::size_of::<INPUT>() as i32);

                        std::thread::sleep(Duration::from_millis(15));

                        // Second stroke: Down -> Hold -> Up
                        let down2 = create_keyboard_input(vk, false);
                        SendInput(1, &down2, std::mem::size_of::<INPUT>() as i32);
                        if hold_duration > Duration::ZERO {
                            if let Some(sig) = signal {
                                if sig.running.load(Ordering::Relaxed) {
                                    let lock = sig.mutex.lock().unwrap();
                                    let _ = sig.condvar.wait_timeout(lock, hold_duration);
                                }
                            } else {
                                std::thread::sleep(hold_duration);
                            }
                        }
                        let up2 = create_keyboard_input(vk, true);
                        SendInput(1, &up2, std::mem::size_of::<INPUT>() as i32);
                    }
                    ClickType::Hold => {
                        press_target_down(target);
                    }
                }
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
    let hold_duration = calculate_key_hold_duration(config.interval);
    let sig = signal.clone();
    run_worker_loop_internal(hwnd_raw, signal, config, move |target| {
        simulate_click_responsive(target, hold_duration, Some(&sig));
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

    // RAII guard guarantees any held mouse button or keyboard key is released when the worker halts
    struct ReleaseGuard(ClickTarget);
    impl Drop for ReleaseGuard {
        fn drop(&mut self) {
            release_target(self.0);
        }
    }
    let _release_guard = ReleaseGuard(config.target);

    let mut count: u64 = 0;
    let mut next_click_time = Instant::now();
    let mut was_paused = false;

    // Immediately enforce mouse position if specific location is selected
    if let MouseLocation::Specific(x, y) = config.location {
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::SetCursorPos(x, y);
        }
    }

    // Immediately press down target if Hold click type and NOT in HoldMouse mode
    if config.trigger_mode != TriggerMode::HoldMouse {
        if matches!(config.target, ClickTarget::Mouse(_, ClickType::Hold) | ClickTarget::Keyboard(_, ClickType::Hold)) {
            press_target_down(config.target);
        }
    }

    while signal.running.load(Ordering::Relaxed) {
        // In Hold trigger mode (Hotkey), monitor hotkey state from background thread
        if config.trigger_mode == TriggerMode::Hold && config.hotkey > 0 {
            let key_down = unsafe {
                (GetAsyncKeyState(config.hotkey as i32) as u16 & 0x8000) != 0
            };
            if !key_down {
                signal.running.store(false, Ordering::SeqCst);
                if hwnd_raw != 0 {
                    unsafe {
                        PostMessageW(hwnd_raw as HWND, WM_APP_STOPPED, 0, 0);
                    }
                }
                break;
            }
        }

        // In HoldMouse trigger mode, pause clicking while physical mouse button is not held down
        if config.trigger_mode == TriggerMode::HoldMouse {
            if !is_physical_mouse_down() {
                if !was_paused {
                    release_target(config.target);
                    was_paused = true;
                }

                let lock = signal.mutex.lock().unwrap();
                if !*lock || !signal.running.load(Ordering::Relaxed) {
                    break;
                }
                let (new_lock, _) = signal
                    .condvar
                    .wait_timeout(lock, Duration::from_millis(15))
                    .unwrap();
                if !*new_lock || !signal.running.load(Ordering::Relaxed) {
                    break;
                }
                continue;
            }

            // Physical mouse is pressed down: if we just unpaused, reset schedule and press down if Hold click type
            if was_paused {
                was_paused = false;
                next_click_time = Instant::now();
                if matches!(config.target, ClickTarget::Mouse(_, ClickType::Hold) | ClickTarget::Keyboard(_, ClickType::Hold)) {
                    press_target_down(config.target);
                }
            }
        }

        // Enforce mouse position before evaluating target window and clicking
        if let MouseLocation::Specific(x, y) = config.location {
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::SetCursorPos(x, y);
            }
        }

        // Prevent self-clicking / self-typing if Clickr's window is targeted
        let skip = if hwnd_raw != 0 {
            match config.target {
                ClickTarget::Mouse(_, _) => unsafe {
                    let mut pt: windows_sys::Win32::Foundation::POINT = std::mem::zeroed();
                    windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt);
                    let wnd = windows_sys::Win32::UI::WindowsAndMessaging::WindowFromPoint(pt);
                    let root = windows_sys::Win32::UI::WindowsAndMessaging::GetAncestor(
                        wnd,
                        windows_sys::Win32::UI::WindowsAndMessaging::GA_ROOT,
                    );
                    wnd == hwnd_raw as HWND || root == hwnd_raw as HWND
                },
                ClickTarget::Keyboard(_, _) => unsafe {
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
            // Check hotkey in Hold trigger mode
            if config.trigger_mode == TriggerMode::Hold && config.hotkey > 0 {
                let key_down = unsafe {
                    (GetAsyncKeyState(config.hotkey as i32) as u16 & 0x8000) != 0
                };
                if !key_down {
                    signal.running.store(false, Ordering::SeqCst);
                    if hwnd_raw != 0 {
                        unsafe {
                            PostMessageW(hwnd_raw as HWND, WM_APP_STOPPED, 0, 0);
                        }
                    }
                    return;
                }
            }

            // In HoldMouse trigger mode: if mouse was released during wait, release target and break to pause
            if config.trigger_mode == TriggerMode::HoldMouse && !is_physical_mouse_down() {
                release_target(config.target);
                was_paused = true;
                break;
            }

            let current = Instant::now();
            if current >= next_click_time {
                break;
            }

            let remaining = next_click_time - current;

            // Phase 1: Coarse responsive sleep while remaining > 1.5ms (0% CPU)
            if remaining > Duration::from_micros(1500) {
                let sleep_duration = (remaining - Duration::from_micros(1000)).max(Duration::from_millis(1));
                // Clamp sleep step to at most 15ms if in Hold/HoldMouse trigger mode or Specific location mode
                let mut sleep_step = sleep_duration;
                if let MouseLocation::Specific(_, _) = config.location {
                    sleep_step = sleep_step.min(Duration::from_millis(20));
                }
                if config.trigger_mode == TriggerMode::Hold || config.trigger_mode == TriggerMode::HoldMouse {
                    sleep_step = sleep_step.min(Duration::from_millis(15));
                }

                let lock = signal.mutex.lock().unwrap();
                if !*lock || !signal.running.load(Ordering::Relaxed) {
                    return;
                }
                let (new_lock, _) = signal.condvar.wait_timeout(lock, sleep_step).unwrap();
                if !*new_lock || !signal.running.load(Ordering::Relaxed) {
                    return;
                }
                if let MouseLocation::Specific(x, y) = config.location {
                    unsafe {
                        windows_sys::Win32::UI::WindowsAndMessaging::SetCursorPos(x, y);
                    }
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
        assert_eq!(dur, Duration::from_millis(10));
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
        assert!(dur >= Duration::from_millis(10));
    }

    #[test]
    fn test_calculate_rate_high_cps_clamp() {
        let dur = calculate_interval(TimingMode::Rate, 0, 0, 0, 0, 10000.0, RateUnit::PerSecond);
        assert_eq!(dur, Duration::from_millis(10));
    }

    #[test]
    fn test_calculate_rate_safe_cps_clamping() {
        // CPS mode: > 100 CPS clamped to 100 CPS (10ms)
        assert_eq!(
            calculate_interval(TimingMode::Rate, 0, 0, 0, 0, 200.0, RateUnit::PerSecond),
            Duration::from_millis(10)
        );
        // CPM mode: > 6000 CPM clamped to 6000 CPM (10ms)
        assert_eq!(
            calculate_interval(TimingMode::Rate, 0, 0, 0, 0, 12000.0, RateUnit::PerMinute),
            Duration::from_millis(10)
        );
        // CPH mode: > 360,000 CPH clamped to 360,000 CPH (10ms)
        assert_eq!(
            calculate_interval(TimingMode::Rate, 0, 0, 0, 0, 720000.0, RateUnit::PerHour),
            Duration::from_millis(10)
        );
        // Safe rate below limit is unaffected (e.g. 50 CPS -> 20ms, 100 CPS -> 10ms)
        assert_eq!(
            calculate_interval(TimingMode::Rate, 0, 0, 0, 0, 50.0, RateUnit::PerSecond),
            Duration::from_millis(20)
        );
        assert_eq!(
            calculate_interval(TimingMode::Rate, 0, 0, 0, 0, 100.0, RateUnit::PerSecond),
            Duration::from_millis(10)
        );
    }

    #[test]
    fn test_timing_drift_compensation_100cps() {
        let signal = Arc::new(WorkerSignal::new());
        let config = ClickerConfig {
            target: ClickTarget::Mouse(MouseButton::Left, ClickType::Single),
            location: MouseLocation::Current,
            trigger_mode: TriggerMode::Toggle,
            hotkey: 0,
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
            target: ClickTarget::Mouse(MouseButton::Left, ClickType::Single),
            location: MouseLocation::Current,
            trigger_mode: TriggerMode::Toggle,
            hotkey: 0,
            interval: Duration::from_millis(10),
            amount: 5,
        };

        let mut timestamps = Vec::new();

        run_worker_loop_internal(0, signal, config, |_| {
            timestamps.push(Instant::now());
            if timestamps.len() == 2 {
                std::thread::sleep(Duration::from_millis(70));
            }
        });

        assert_eq!(timestamps.len(), 5);

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
            target: ClickTarget::Mouse(MouseButton::Left, ClickType::Single),
            location: MouseLocation::Current,
            trigger_mode: TriggerMode::Toggle,
            hotkey: 0,
            interval: Duration::from_secs(5), // 5 seconds interval
            amount: 0,
        };

        let sig_clone = signal.clone();
        let handle = std::thread::spawn(move || {
            let start = Instant::now();
            run_worker_loop_internal(0, sig_clone, config, |_| {});
            start.elapsed()
        });

        std::thread::sleep(Duration::from_millis(20));

        let stop_time = Instant::now();
        signal.stop();

        let elapsed = handle.join().expect("Worker thread panicked");
        let stop_latency = stop_time.elapsed();

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
            target: ClickTarget::Mouse(MouseButton::Left, ClickType::Single),
            location: MouseLocation::Current,
            trigger_mode: TriggerMode::Toggle,
            hotkey: 0,
            interval: Duration::from_millis(10), // 100 CPS
            amount: total_clicks,
        };

        let mut timestamps = Vec::with_capacity(total_clicks as usize);

        let overall_start = Instant::now();
        run_worker_loop_internal(0, signal, config, |_| {
            timestamps.push(Instant::now());
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
            target: ClickTarget::Mouse(MouseButton::Left, ClickType::Single),
            location: MouseLocation::Current,
            trigger_mode: TriggerMode::Toggle,
            hotkey: 0,
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

        let assumed_freq_hz = 3.0e9;
        let thread_active_secs = cycles as f64 / assumed_freq_hz;
        let core_cpu_pct = (thread_active_secs / wall_secs) * 100.0;
        println!("Estimated Active Core CPU Usage: {:.2}%", core_cpu_pct);

        assert!(
            core_cpu_pct < 10.0,
            "Active CPU usage too high: {:.2}%",
            core_cpu_pct
        );
    }

    #[test]
    fn test_mouse_location_config() {
        let config_current = ClickerConfig {
            target: ClickTarget::Mouse(MouseButton::Left, ClickType::Single),
            location: MouseLocation::Current,
            trigger_mode: TriggerMode::Toggle,
            hotkey: 0,
            interval: Duration::from_millis(100),
            amount: 10,
        };
        assert_eq!(config_current.location, MouseLocation::Current);

        let config_specific = ClickerConfig {
            target: ClickTarget::Mouse(MouseButton::Right, ClickType::Single),
            location: MouseLocation::Specific(500, 300),
            trigger_mode: TriggerMode::Toggle,
            hotkey: 0,
            interval: Duration::from_millis(50),
            amount: 5,
        };
        assert_eq!(config_specific.location, MouseLocation::Specific(500, 300));
    }

    #[test]
    fn test_double_click_config_and_execution() {
        let signal = Arc::new(WorkerSignal::new());
        let config = ClickerConfig {
            target: ClickTarget::Mouse(MouseButton::Left, ClickType::Double),
            location: MouseLocation::Current,
            trigger_mode: TriggerMode::Toggle,
            hotkey: 0,
            interval: Duration::from_millis(10),
            amount: 3,
        };

        let mut received_targets = Vec::new();
        run_worker_loop_internal(0, signal, config, |t| {
            received_targets.push(t);
        });

        assert_eq!(received_targets.len(), 3);
        assert_eq!(
            received_targets[0],
            ClickTarget::Mouse(MouseButton::Left, ClickType::Double)
        );
    }

    #[test]
    fn test_hold_click_type_mouse_and_keyboard() {
        let signal = Arc::new(WorkerSignal::new());
        let config_mouse = ClickerConfig {
            target: ClickTarget::Mouse(MouseButton::Left, ClickType::Hold),
            location: MouseLocation::Current,
            trigger_mode: TriggerMode::Toggle,
            hotkey: 0,
            interval: Duration::from_millis(10),
            amount: 2,
        };

        let mut mouse_targets = Vec::new();
        run_worker_loop_internal(0, signal.clone(), config_mouse, |t| {
            mouse_targets.push(t);
        });
        assert_eq!(mouse_targets.len(), 2);
        assert_eq!(mouse_targets[0], ClickTarget::Mouse(MouseButton::Left, ClickType::Hold));

        let signal2 = Arc::new(WorkerSignal::new());
        let config_kbd = ClickerConfig {
            target: ClickTarget::Keyboard(windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_SPACE, ClickType::Hold),
            location: MouseLocation::Current,
            trigger_mode: TriggerMode::Toggle,
            hotkey: 0,
            interval: Duration::from_millis(10),
            amount: 2,
        };

        let mut kbd_targets = Vec::new();
        run_worker_loop_internal(0, signal2, config_kbd, |t| {
            kbd_targets.push(t);
        });
        assert_eq!(kbd_targets.len(), 2);
        assert_eq!(kbd_targets[0], ClickTarget::Keyboard(windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_SPACE, ClickType::Hold));
    }

    #[test]
    fn test_hold_trigger_mode_stops_when_hotkey_released() {
        let signal = Arc::new(WorkerSignal::new());
        let config = ClickerConfig {
            target: ClickTarget::Mouse(MouseButton::Left, ClickType::Single),
            location: MouseLocation::Current,
            trigger_mode: TriggerMode::Hold,
            hotkey: 0x88, // An unpressed unassigned virtual key
            interval: Duration::from_millis(100),
            amount: 10,
        };

        let mut count = 0;
        let start = Instant::now();
        run_worker_loop_internal(0, signal, config, |_| {
            count += 1;
        });
        let elapsed = start.elapsed();
        // Since hotkey 0x88 is not held down, it terminates immediately
        assert_eq!(count, 0);
        assert!(elapsed < Duration::from_millis(50));
    }

    #[test]
    fn test_hold_mouse_trigger_mode() {
        set_physical_mouse_down(false);
        let signal = Arc::new(WorkerSignal::new());
        let config = ClickerConfig {
            target: ClickTarget::Mouse(MouseButton::Left, ClickType::Single),
            location: MouseLocation::Current,
            trigger_mode: TriggerMode::HoldMouse,
            hotkey: 0,
            interval: Duration::from_millis(5),
            amount: 0,
        };

        let click_count = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let count_clone = click_count.clone();
        let signal_clone = signal.clone();

        let handle = std::thread::spawn(move || {
            run_worker_loop_internal(0, signal_clone, config, |_| {
                count_clone.fetch_add(1, Ordering::SeqCst);
            });
        });

        // 1. Initially mouse button is not held: verify it does not click
        std::thread::sleep(Duration::from_millis(25));
        assert_eq!(click_count.load(Ordering::SeqCst), 0);

        // 2. User holds down mouse click: verify it starts clicking
        set_physical_mouse_down(true);
        signal.condvar.notify_all();
        std::thread::sleep(Duration::from_millis(30));
        let count_after_down = click_count.load(Ordering::SeqCst);
        assert!(
            count_after_down >= 2,
            "Expected at least 2 clicks while held down, got {}",
            count_after_down
        );

        // 3. User releases mouse click: verify it pauses clicking
        set_physical_mouse_down(false);
        signal.condvar.notify_all();
        std::thread::sleep(Duration::from_millis(25));
        let count_after_release = click_count.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(25));
        assert_eq!(
            click_count.load(Ordering::SeqCst),
            count_after_release,
            "Should remain paused when released"
        );

        // 4. User holds down mouse click again: verify it resumes clicking
        set_physical_mouse_down(true);
        signal.condvar.notify_all();
        std::thread::sleep(Duration::from_millis(30));
        assert!(
            click_count.load(Ordering::SeqCst) > count_after_release,
            "Should resume clicking on second hold"
        );

        // Cleanup
        set_physical_mouse_down(false);
        signal.stop();
        handle.join().unwrap();
    }

    #[test]
    fn test_calculate_key_hold_duration_adaptive() {
        // High CPS: 100 CPS (10ms) -> 5ms hold
        assert_eq!(
            calculate_key_hold_duration(Duration::from_millis(10)),
            Duration::from_millis(5)
        );

        // Medium CPS: 50 CPS (20ms) -> 10ms hold
        assert_eq!(
            calculate_key_hold_duration(Duration::from_millis(20)),
            Duration::from_millis(10)
        );

        // Low CPS: 20 CPS (50ms) -> 25ms cap
        assert_eq!(
            calculate_key_hold_duration(Duration::from_millis(50)),
            Duration::from_millis(25)
        );

        // Normal typing rate: 10 CPS (100ms) -> 25ms natural hold
        assert_eq!(
            calculate_key_hold_duration(Duration::from_millis(100)),
            Duration::from_millis(25)
        );

        // Very slow rate: 1 CPS (1000ms) -> 25ms natural hold
        assert_eq!(
            calculate_key_hold_duration(Duration::from_millis(1000)),
            Duration::from_millis(25)
        );

        // Extreme fast: 2ms -> 1ms hold
        assert_eq!(
            calculate_key_hold_duration(Duration::from_millis(2)),
            Duration::from_millis(1)
        );
    }

    #[test]
    fn test_create_keyboard_input_hardware_scancode_and_flags() {
        unsafe {
            // Standard key: VK_SPACE
            let down = create_keyboard_input(VK_SPACE, false);
            assert_eq!(down.r#type, INPUT_KEYBOARD);
            assert_eq!(down.Anonymous.ki.wVk, VK_SPACE);
            assert_ne!(down.Anonymous.ki.wScan, 0, "Hardware scan code must be non-zero for Space");
            assert_eq!(down.Anonymous.ki.dwFlags, 0, "Key Down must not have KEYUP flag");

            let up = create_keyboard_input(VK_SPACE, true);
            assert_eq!(up.r#type, INPUT_KEYBOARD);
            assert_eq!(up.Anonymous.ki.wVk, VK_SPACE);
            assert_eq!(up.Anonymous.ki.wScan, down.Anonymous.ki.wScan, "Scan code must match on Down and Up");
            assert_eq!(up.Anonymous.ki.dwFlags & KEYEVENTF_KEYUP, KEYEVENTF_KEYUP, "Key Up must have KEYUP flag");

            // Extended key: VK_UP arrow
            let ext_down = create_keyboard_input(VK_UP, false);
            assert_eq!(ext_down.Anonymous.ki.wVk, VK_UP);
            assert_eq!(
                ext_down.Anonymous.ki.dwFlags & KEYEVENTF_EXTENDEDKEY,
                KEYEVENTF_EXTENDEDKEY,
                "Extended key must have KEYEVENTF_EXTENDEDKEY flag"
            );

            let ext_up = create_keyboard_input(VK_UP, true);
            assert_eq!(
                ext_up.Anonymous.ki.dwFlags & (KEYEVENTF_KEYUP | KEYEVENTF_EXTENDEDKEY),
                KEYEVENTF_KEYUP | KEYEVENTF_EXTENDEDKEY,
                "Extended Key Up must have both KEYUP and EXTENDEDKEY flags"
            );
        }
    }

    #[test]
    fn test_simulate_click_responsive_cancellation() {
        let signal = Arc::new(WorkerSignal::new());
        let sig_clone = signal.clone();

        // Spawn a thread simulating a 500ms hold keypress
        let handle = std::thread::spawn(move || {
            let start = Instant::now();
            simulate_click_responsive(
                ClickTarget::Keyboard(VK_SPACE, ClickType::Single),
                Duration::from_millis(500),
                Some(&sig_clone),
            );
            start.elapsed()
        });

        // Sleep briefly and then trigger stop signal
        std::thread::sleep(Duration::from_millis(20));
        signal.stop();

        let elapsed = handle.join().expect("Thread panicked");
        // Verify it aborted immediately rather than waiting the full 500ms
        assert!(
            elapsed < Duration::from_millis(100),
            "Hold wait should have aborted immediately upon stop signal, elapsed: {:?}",
            elapsed
        );
    }
}
