use std::ffi::c_void;
use std::ptr::null;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, HWND, POINT, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{
    SetEvent, SetWaitableTimer, WaitForMultipleObjects, CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
    INFINITE, TIMER_ALL_ACCESS,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetCursorPos, GetForegroundWindow, PostMessageW, SetCursorPos, WindowFromPoint,
    GA_ROOT,
};

#[link(name = "kernel32")]
extern "system" {
    fn CreateEventW(
        lpEventAttributes: *const c_void,
        bManualReset: i32,
        bInitialState: i32,
        lpName: *const u16,
    ) -> HANDLE;
    fn CreateWaitableTimerExW(
        lpTimerAttributes: *const c_void,
        lpTimerName: *const u16,
        dwFlags: u32,
        dwDesiredAccess: u32,
    ) -> HANDLE;
}

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
    pub fn from_index(index: usize) -> Self {
        match index {
            1 => RateUnit::PerMinute,
            2 => RateUnit::PerHour,
            _ => RateUnit::PerSecond,
        }
    }

    pub fn unit_seconds(&self) -> f64 {
        match self {
            RateUnit::PerSecond => 1.0,
            RateUnit::PerMinute => 60.0,
            RateUnit::PerHour => 3600.0,
        }
    }

    pub fn max_safe_rate(&self) -> f64 {
        MAX_SAFE_CPS * self.unit_seconds()
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
    PHYSICAL_MOUSE_DOWN.load(Ordering::Acquire)
}

pub fn set_physical_mouse_down(down: bool) {
    PHYSICAL_MOUSE_DOWN.store(down, Ordering::Release);
}

pub static WINDOW_RECT_LEFT: AtomicI32 = AtomicI32::new(0);
pub static WINDOW_RECT_TOP: AtomicI32 = AtomicI32::new(0);
pub static WINDOW_RECT_RIGHT: AtomicI32 = AtomicI32::new(0);
pub static WINDOW_RECT_BOTTOM: AtomicI32 = AtomicI32::new(0);

pub fn update_window_bounds(left: i32, top: i32, right: i32, bottom: i32) {
    WINDOW_RECT_LEFT.store(left, Ordering::Release);
    WINDOW_RECT_TOP.store(top, Ordering::Release);
    WINDOW_RECT_RIGHT.store(right, Ordering::Release);
    WINDOW_RECT_BOTTOM.store(bottom, Ordering::Release);
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

struct OwnedHandle(HANDLE);

unsafe impl Send for OwnedHandle {}
unsafe impl Sync for OwnedHandle {}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

/// High-resolution Win32 waitable timer with automatic fallback on older Windows versions.
pub struct WaitableTimer {
    handle: OwnedHandle,
    is_high_res: bool,
}

impl WaitableTimer {
    pub fn new() -> Self {
        let (handle, is_high_res) = unsafe {
            let h = CreateWaitableTimerExW(
                null(),
                null(),
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                TIMER_ALL_ACCESS,
            );
            if h.is_null() {
                (CreateWaitableTimerExW(null(), null(), 0, TIMER_ALL_ACCESS), false)
            } else {
                (h, true)
            }
        };
        Self {
            handle: OwnedHandle(handle),
            is_high_res,
        }
    }

    fn arm(&self, duration: Duration) -> bool {
        if self.handle.0.is_null() {
            return false;
        }
        let ticks_100ns = (duration.as_nanos() / 100).max(1).min(i64::MAX as u128) as i64;
        let due_time = -ticks_100ns;
        unsafe { SetWaitableTimer(self.handle.0, &due_time, 0, None, null(), 0) != 0 }
    }
}

pub struct WorkerSignal {
    pub running: AtomicBool,
    stop_event: OwnedHandle,
    wake_event: OwnedHandle,
}

impl WorkerSignal {
    pub fn new() -> Self {
        let stop_event = unsafe { CreateEventW(null(), 1, 0, null()) };
        let wake_event = unsafe { CreateEventW(null(), 0, 0, null()) };
        Self {
            running: AtomicBool::new(true),
            stop_event: OwnedHandle(stop_event),
            wake_event: OwnedHandle(wake_event),
        }
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        if !self.stop_event.0.is_null() {
            unsafe {
                SetEvent(self.stop_event.0);
            }
        }
    }

    /// Wake any thread waiting on this signal (e.g. physical mouse state change).
    pub fn notify(&self) {
        if !self.wake_event.0.is_null() {
            unsafe {
                SetEvent(self.wake_event.0);
            }
        }
    }

    /// Raw Win32 wake event handle for lock-free signaling from low-level hooks.
    pub fn wake_handle_raw(&self) -> isize {
        self.wake_event.0 as isize
    }

    /// Wait until either `stop()` is called, `notify()` is called, or `duration` elapses.
    /// Returns `false` if the worker was stopped.
    fn wait_for_timer_or_wake(&self, duration: Duration, timer: Option<&WaitableTimer>) -> bool {
        if !self.running.load(Ordering::Acquire) {
            return false;
        }
        if let Some(t) = timer {
            if !self.stop_event.0.is_null() && !self.wake_event.0.is_null() && t.arm(duration) {
                let handles = [self.stop_event.0, self.wake_event.0, t.handle.0];
                let res = unsafe { WaitForMultipleObjects(3, handles.as_ptr(), 0, INFINITE) };
                return res != WAIT_OBJECT_0 && self.running.load(Ordering::Acquire);
            }
        }
        if !self.stop_event.0.is_null() && !self.wake_event.0.is_null() {
            let handles = [self.stop_event.0, self.wake_event.0];
            let ms = duration.as_millis().max(1).min(u32::MAX as u128) as u32;
            let res = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, ms) };
            return res != WAIT_OBJECT_0 && self.running.load(Ordering::Acquire);
        }
        std::thread::sleep(duration);
        self.running.load(Ordering::Acquire)
    }

    /// Wait until `stop()` or `notify()` (or safety timeout) while paused in HoldMouse mode.
    fn wait_while_paused(&self) -> bool {
        if !self.running.load(Ordering::Acquire) {
            return false;
        }
        if !self.stop_event.0.is_null() && !self.wake_event.0.is_null() {
            let handles = [self.stop_event.0, self.wake_event.0];
            let res = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, 50) };
            return res != WAIT_OBJECT_0 && self.running.load(Ordering::Acquire);
        }
        std::thread::sleep(Duration::from_millis(15));
        self.running.load(Ordering::Acquire)
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

#[inline]
fn make_mouse_input(flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

#[inline]
fn mouse_button_flags(btn: MouseButton) -> (u32, u32) {
    match btn {
        MouseButton::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
        MouseButton::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
        MouseButton::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
    }
}

#[inline]
fn send_inputs(inputs: &[INPUT]) {
    unsafe {
        SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
    }
}

/// Press a target mouse button or keyboard key down (used for Hold click type).
pub fn press_target_down(target: ClickTarget) {
    let input = match target {
        ClickTarget::Mouse(btn, _) => make_mouse_input(mouse_button_flags(btn).0),
        ClickTarget::Keyboard(vk, _) => create_keyboard_input(vk, false),
    };
    send_inputs(std::slice::from_ref(&input));
}

/// Release a target mouse button or keyboard key up (used for Hold click type).
pub fn release_target(target: ClickTarget) {
    let input = match target {
        ClickTarget::Mouse(btn, _) => make_mouse_input(mouse_button_flags(btn).1),
        ClickTarget::Keyboard(vk, _) => create_keyboard_input(vk, true),
    };
    send_inputs(std::slice::from_ref(&input));
}

/// Simulate a single/double click or hold state for mouse or keyboard with default hold time.
pub fn simulate_click(target: ClickTarget) {
    simulate_click_responsive(target, Duration::from_millis(20), None);
}

/// Simulate a single/double click or hold state with a specific hold duration.
pub fn simulate_click_with_hold(target: ClickTarget, hold_duration: Duration) {
    simulate_click_responsive(target, hold_duration, None);
}

#[inline]
fn wait_responsive(
    duration: Duration,
    signal: Option<&WorkerSignal>,
    timer: Option<&WaitableTimer>,
) -> bool {
    if let Some(sig) = signal {
        sig.wait_for_timer_or_wake(duration, timer)
    } else {
        std::thread::sleep(duration);
        true
    }
}

#[inline]
fn send_key_press_and_release(
    down: &INPUT,
    up: &INPUT,
    hold_duration: Duration,
    signal: Option<&WorkerSignal>,
    timer: Option<&WaitableTimer>,
) {
    send_inputs(std::slice::from_ref(down));
    if hold_duration > Duration::ZERO {
        let _ = wait_responsive(hold_duration, signal, timer);
    }
    send_inputs(std::slice::from_ref(up));
}

pub fn simulate_keyboard_click_precomputed(
    down: INPUT,
    up: INPUT,
    click_type: ClickType,
    hold_duration: Duration,
    signal: Option<&WorkerSignal>,
    timer: Option<&WaitableTimer>,
) {
    match click_type {
        ClickType::Single => {
            send_key_press_and_release(&down, &up, hold_duration, signal, timer);
        }
        ClickType::Double => {
            send_key_press_and_release(&down, &up, hold_duration, signal, timer);
            if !wait_responsive(Duration::from_millis(15), signal, timer) {
                return;
            }
            send_key_press_and_release(&down, &up, hold_duration, signal, timer);
        }
        ClickType::Hold => {
            send_inputs(std::slice::from_ref(&down));
        }
    }
}

#[inline]
fn simulate_mouse_click_precomputed(
    inputs: &[INPUT; 2],
    click_type: ClickType,
    signal: Option<&WorkerSignal>,
    timer: Option<&WaitableTimer>,
) {
    match click_type {
        ClickType::Single => {
            send_inputs(inputs);
        }
        ClickType::Double => {
            send_inputs(inputs);
            // Inter-click interval for Windows to recognize distinct double-click events
            if !wait_responsive(Duration::from_millis(15), signal, timer) {
                return;
            }
            send_inputs(inputs);
        }
        ClickType::Hold => {
            send_inputs(std::slice::from_ref(&inputs[0]));
        }
    }
}

/// Simulate click with adaptive down-to-up hold delay and responsive cancellation on worker stop.
pub fn simulate_click_responsive(
    target: ClickTarget,
    hold_duration: Duration,
    signal: Option<&WorkerSignal>,
) {
    match target {
        ClickTarget::Mouse(btn, click_type) => {
            let (down_flag, up_flag) = mouse_button_flags(btn);
            let inputs = [make_mouse_input(down_flag), make_mouse_input(up_flag)];
            simulate_mouse_click_precomputed(&inputs, click_type, signal, None);
        }
        ClickTarget::Keyboard(vk, click_type) => {
            let down = create_keyboard_input(vk, false);
            let up = create_keyboard_input(vk, true);
            simulate_keyboard_click_precomputed(down, up, click_type, hold_duration, signal, None);
        }
    }
}

/// Background worker loop that executes clicks with per-click drift compensation
/// and a low-CPU hybrid wait (high-resolution waitable timer + microsecond spin-wait).
/// Halts immediately when signaled without waiting for the sleep duration to expire.
pub fn run_worker_loop(
    hwnd_raw: isize,
    signal: Arc<WorkerSignal>,
    config: ClickerConfig,
) {
    let hold_duration = calculate_key_hold_duration(config.interval);
    let sig = signal.clone();

    // Precompute INPUT structs outside the hot loop so MapVirtualKeyW / flag mapping is not repeated on every tick
    enum PrecomputedTarget {
        Mouse([INPUT; 2], ClickType),
        Keyboard(INPUT, INPUT, ClickType),
        HoldOnly,
    }

    let precomputed = match config.target {
        ClickTarget::Mouse(_, ClickType::Hold) | ClickTarget::Keyboard(_, ClickType::Hold) => {
            PrecomputedTarget::HoldOnly
        }
        ClickTarget::Mouse(btn, click_type) => {
            let (down_flag, up_flag) = mouse_button_flags(btn);
            PrecomputedTarget::Mouse(
                [make_mouse_input(down_flag), make_mouse_input(up_flag)],
                click_type,
            )
        }
        ClickTarget::Keyboard(vk, click_type) => PrecomputedTarget::Keyboard(
            create_keyboard_input(vk, false),
            create_keyboard_input(vk, true),
            click_type,
        ),
    };

    run_worker_loop_internal_with_timer(hwnd_raw, signal, config, move |_, timer| {
        match &precomputed {
            PrecomputedTarget::Mouse(inputs, click_type) => {
                simulate_mouse_click_precomputed(inputs, *click_type, Some(&sig), Some(timer));
            }
            PrecomputedTarget::Keyboard(down, up, click_type) => {
                simulate_keyboard_click_precomputed(
                    *down,
                    *up,
                    *click_type,
                    hold_duration,
                    Some(&sig),
                    Some(timer),
                );
            }
            PrecomputedTarget::HoldOnly => {}
        }
    });
}

#[inline]
fn notify_ui_stopped(hwnd_raw: isize, signal: &WorkerSignal) {
    signal.stop();
    if hwnd_raw != 0 {
        unsafe {
            PostMessageW(hwnd_raw as HWND, WM_APP_STOPPED, 0, 0);
        }
    }
}

#[inline]
fn check_hold_hotkey_released(hwnd_raw: isize, signal: &WorkerSignal, config: &ClickerConfig) -> bool {
    if config.trigger_mode == TriggerMode::Hold && config.hotkey > 0 {
        let key_down = unsafe { (GetAsyncKeyState(config.hotkey as i32) as u16 & 0x8000) != 0 };
        if !key_down {
            notify_ui_stopped(hwnd_raw, signal);
            return true;
        }
    }
    false
}

#[inline]
fn should_skip_self_click(hwnd_raw: isize, config: &ClickerConfig) -> bool {
    if hwnd_raw == 0 {
        return false;
    }
    let target_hwnd = hwnd_raw as HWND;
    match config.target {
        ClickTarget::Mouse(_, _) => {
            let pt = match config.location {
                MouseLocation::Specific(x, y) => POINT { x, y },
                MouseLocation::Current => unsafe {
                    let mut pt: POINT = std::mem::zeroed();
                    GetCursorPos(&mut pt);
                    pt
                },
            };

            let left = WINDOW_RECT_LEFT.load(Ordering::Relaxed);
            let top = WINDOW_RECT_TOP.load(Ordering::Relaxed);
            let right = WINDOW_RECT_RIGHT.load(Ordering::Relaxed);
            let bottom = WINDOW_RECT_BOTTOM.load(Ordering::Relaxed);

            if pt.x >= left && pt.x < right && pt.y >= top && pt.y < bottom {
                unsafe {
                    let wnd = WindowFromPoint(pt);
                    wnd == target_hwnd || GetAncestor(wnd, GA_ROOT) == target_hwnd
                }
            } else {
                false
            }
        }
        ClickTarget::Keyboard(_, _) => unsafe {
            let foreground = GetForegroundWindow();
            foreground == target_hwnd || GetAncestor(foreground, GA_ROOT) == target_hwnd
        },
    }
}

/// Internal loop implementation accepting a click callback, enabling precise unit testing
/// and timing verification without emitting real desktop mouse/keyboard events.
pub fn run_worker_loop_internal<F: FnMut(ClickTarget)>(
    hwnd_raw: isize,
    signal: Arc<WorkerSignal>,
    config: ClickerConfig,
    mut click_fn: F,
) {
    run_worker_loop_internal_with_timer(hwnd_raw, signal, config, move |target, _| {
        click_fn(target);
    });
}

fn run_worker_loop_internal_with_timer<F: FnMut(ClickTarget, &WaitableTimer)>(
    hwnd_raw: isize,
    signal: Arc<WorkerSignal>,
    config: ClickerConfig,
    mut click_fn: F,
) {
    // Enable 1ms Windows timer resolution during clicking via RAII guard
    let _timer_guard = TimerResolutionGuard::new();
    let timer = WaitableTimer::new();

    let is_hold = matches!(
        config.target,
        ClickTarget::Mouse(_, ClickType::Hold) | ClickTarget::Keyboard(_, ClickType::Hold)
    );

    // RAII guard guarantees any held mouse button or keyboard key is released when the worker halts
    struct ReleaseGuard {
        target: ClickTarget,
        is_held: bool,
    }
    impl ReleaseGuard {
        #[inline]
        fn press(&mut self) {
            if !self.is_held {
                press_target_down(self.target);
                self.is_held = true;
            }
        }
        #[inline]
        fn release(&mut self) {
            if self.is_held {
                release_target(self.target);
                self.is_held = false;
            }
        }
    }
    impl Drop for ReleaseGuard {
        fn drop(&mut self) {
            self.release();
        }
    }

    let mut release_guard = ReleaseGuard {
        target: config.target,
        is_held: false,
    };

    let mut count: u64 = 0;
    let mut next_click_time = Instant::now();
    let mut was_paused = false;

    // Immediately press down target if Hold click type and NOT in HoldMouse mode
    if is_hold && config.trigger_mode != TriggerMode::HoldMouse {
        if let MouseLocation::Specific(x, y) = config.location {
            unsafe {
                SetCursorPos(x, y);
            }
        }
        release_guard.press();
    }

    while signal.running.load(Ordering::Relaxed) {
        if check_hold_hotkey_released(hwnd_raw, &signal, &config) {
            break;
        }

        // In HoldMouse trigger mode, pause clicking while physical mouse button is not held down
        if config.trigger_mode == TriggerMode::HoldMouse {
            if !is_physical_mouse_down() {
                if !was_paused {
                    release_guard.release();
                    was_paused = true;
                }

                if !signal.wait_while_paused() {
                    break;
                }
                continue;
            }

            // Physical mouse is pressed down: if we just unpaused, reset schedule and press down if Hold click type
            if was_paused {
                was_paused = false;
                next_click_time = Instant::now();
                if is_hold {
                    release_guard.press();
                }
            }
        }

        // Enforce mouse position before evaluating target window and clicking
        if let MouseLocation::Specific(x, y) = config.location {
            unsafe {
                SetCursorPos(x, y);
            }
        }

        if !should_skip_self_click(hwnd_raw, &config) {
            if is_hold {
                release_guard.press();
            }
            click_fn(config.target, &timer);
            count += 1;

            if config.amount > 0 && count >= config.amount {
                notify_ui_stopped(hwnd_raw, &signal);
                break;
            }
        }

        // 1. Advance the target schedule by the exact interval
        next_click_time += config.interval;

        // 2. Lag-Spike / Buffer Guard
        let now = Instant::now();
        if now > next_click_time {
            let lag = now.duration_since(next_click_time);
            let max_allowed_lag = (config.interval * 5).max(Duration::from_millis(50));
            if lag > max_allowed_lag {
                next_click_time = now;
            }
        }

        // 3. Low-CPU Hybrid Wait for remaining time:
        //    High-res waitable timer sleeps until ~150us before target; legacy timer falls back to 1ms margin.
        let (spin_threshold, timer_margin) = if timer.is_high_res {
            (Duration::from_micros(250), Duration::from_micros(150))
        } else {
            (Duration::from_micros(1500), Duration::from_micros(1000))
        };

        while signal.running.load(Ordering::Relaxed) {
            if check_hold_hotkey_released(hwnd_raw, &signal, &config) {
                return;
            }

            // In HoldMouse trigger mode: if mouse was released during wait, release target and break to pause
            if config.trigger_mode == TriggerMode::HoldMouse && !is_physical_mouse_down() {
                release_guard.release();
                was_paused = true;
                break;
            }

            let current = Instant::now();
            if current >= next_click_time {
                break;
            }

            let remaining = next_click_time - current;

            if remaining > spin_threshold {
                let mut sleep_step = remaining - timer_margin;
                if config.trigger_mode == TriggerMode::Hold {
                    sleep_step = sleep_step.min(Duration::from_millis(15));
                }

                if !signal.wait_for_timer_or_wake(sleep_step, Some(&timer)) {
                    return;
                }
                continue;
            }

            // Lightweight CPU pause spin-wait for final sub-millisecond window
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
        signal.notify();
        std::thread::sleep(Duration::from_millis(30));
        let count_after_down = click_count.load(Ordering::SeqCst);
        assert!(
            count_after_down >= 2,
            "Expected at least 2 clicks while held down, got {}",
            count_after_down
        );

        // 3. User releases mouse click: verify it pauses clicking
        set_physical_mouse_down(false);
        signal.notify();
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
        signal.notify();
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
