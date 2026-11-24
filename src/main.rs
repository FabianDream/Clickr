#![windows_subsystem = "windows"]

pub mod clicker;
pub mod hotkeys;
pub mod settings;
pub mod ui;
pub mod updater;

use std::ptr::{null, null_mut};
use std::sync::Arc;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
use windows_sys::Win32::UI::Controls::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use clicker::*;
use hotkeys::*;
use settings::*;
use ui::*;
use updater::{UpdateInfo, UpdateStatus};

pub const WM_APP_UPDATE_AVAILABLE: u32 = WM_USER + 101;
pub const WM_APP_UPDATE_NOT_FOUND: u32 = WM_USER + 102;
pub const WM_APP_UPDATE_CHECK_FAILED: u32 = WM_USER + 103;
pub const WM_APP_UPDATE_FAILED: u32 = WM_USER + 104;
pub const IDM_CHECK_UPDATES: u32 = 2001;

#[link(name = "user32")]
extern "system" {
    fn MessageBeep(uType: u32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetProcessHeap() -> HANDLE;
    fn HeapCompact(hHeap: HANDLE, dwFlags: u32) -> usize;
}

#[link(name = "dwmapi")]
extern "system" {
    fn DwmSetWindowAttribute(
        hwnd: HWND,
        dwAttribute: u32,
        pvAttribute: *const std::ffi::c_void,
        cbAttribute: u32,
    ) -> i32;
}

struct WindowContext {
    hwnd: HWND,
    state: AppState,
    controls: Controls,
    theme: ThemeBrushes,
    font_reg: HFONT,
    font_bold: HFONT,
    last_saved_settings: Option<AppSettings>,
}

fn parse_u64_or_0(s: &str) -> u64 {
    s.trim().parse::<u64>().unwrap_or(0)
}

fn parse_u64_or_default(s: &str, def: u64) -> u64 {
    s.trim().parse::<u64>().unwrap_or(def)
}

fn parse_i32_or_0(s: &str) -> i32 {
    s.trim().parse::<i32>().unwrap_or(0)
}

fn parse_f64_or_default(s: &str, def: f64) -> f64 {
    let s = s.trim();
    if let Ok(val) = s.parse::<f64>() {
        if val > 0.0 && !val.is_nan() && !val.is_infinite() {
            return val;
        }
    }
    def
}

fn non_empty_or(s: String, default: &str) -> String {
    if s.is_empty() { default.to_string() } else { s }
}

fn save_current_settings(ctx: &mut WindowContext) {
    let timing_mode = ctx.state.timing_mode;
    let input_mode = ctx.state.input_mode;
    let location_mode = ctx.state.location_mode;
    let trigger_mode = combobox_get_cursel(ctx.controls.combo_trigger_mode) as u32;

    let rate_unit = combobox_get_cursel(ctx.controls.combo_rate_unit);
    let mouse_btn = combobox_get_cursel(ctx.controls.combo_mouse_btn);
    let click_type = get_click_type_from_combo(ctx.controls.combo_click_type, input_mode);

    let hr = non_empty_or(get_control_text(ctx.controls.edit_hr), "0");
    let min = non_empty_or(get_control_text(ctx.controls.edit_min), "0");
    let sec = non_empty_or(get_control_text(ctx.controls.edit_sec), "0");
    let ms = non_empty_or(get_control_text(ctx.controls.edit_ms), "100");
    let rate = non_empty_or(get_control_text(ctx.controls.edit_rate), "50");
    let click_amount = non_empty_or(get_control_text(ctx.controls.edit_amount), "0");

    let target_x = parse_i32_or_0(&get_control_text(ctx.controls.edit_loc_x));
    let target_y = parse_i32_or_0(&get_control_text(ctx.controls.edit_loc_y));

    let (pos_x, pos_y) = unsafe {
        if IsIconic(ctx.hwnd) == 0 {
            let mut rc: RECT = std::mem::zeroed();
            GetWindowRect(ctx.hwnd, &mut rc);
            (rc.left, rc.top)
        } else {
            (ctx.state.pos_x, ctx.state.pos_y)
        }
    };

    let mut settings = AppSettings {
        hotkey: ctx.state.hotkey,
        target_key: ctx.state.target_key,
        trigger_mode,
        timing_mode,
        hr,
        min,
        sec,
        ms,
        rate,
        rate_unit,
        input_mode,
        mouse_btn,
        click_type,
        location_mode,
        target_x,
        target_y,
        click_amount,
        pos_x,
        pos_y,
    };

    clamp_settings_limits(&mut settings);

    if ctx.last_saved_settings.as_ref() == Some(&settings) {
        return;
    }

    save_settings(&settings);
    ctx.last_saved_settings = Some(settings);
}

fn start_clicker(hwnd: HWND, ctx: &mut WindowContext) {
    if ctx.state.is_running {
        return;
    }

    // Determine timing configuration
    let timing_mode = if ctx.state.timing_mode == 0 {
        TimingMode::Interval
    } else {
        TimingMode::Rate
    };

    let hr = parse_u64_or_0(&get_control_text(ctx.controls.edit_hr));
    let min = parse_u64_or_0(&get_control_text(ctx.controls.edit_min));
    let sec = parse_u64_or_0(&get_control_text(ctx.controls.edit_sec));
    let mut ms = parse_u64_or_default(&get_control_text(ctx.controls.edit_ms), 100);

    // Enforce safe minimum delay (10 ms / 100 CPS) in Interval mode
    if timing_mode == TimingMode::Interval && hr == 0 && min == 0 && sec == 0 && ms < 10 {
        ms = 10;
        set_control_text(ctx.controls.edit_ms, "10");
    }

    let rate_unit = RateUnit::from_index(combobox_get_cursel(ctx.controls.combo_rate_unit));

    let raw_rate = parse_f64_or_default(&get_control_text(ctx.controls.edit_rate), 50.0);
    // Enforce safe maximum rate (100 CPS) in Rate mode to prevent Windows slowdowns and crashes
    let max_rate_for_unit = rate_unit.max_safe_rate();
    let rate = if raw_rate > max_rate_for_unit {
        set_control_text(ctx.controls.edit_rate, &format!("{}", max_rate_for_unit as u64));
        max_rate_for_unit
    } else {
        raw_rate
    };

    let interval = calculate_interval(timing_mode, hr, min, sec, ms, rate, rate_unit);

    let trigger_mode = match combobox_get_cursel(ctx.controls.combo_trigger_mode) {
        1 => TriggerMode::Hold,
        2 => TriggerMode::HoldMouse,
        _ => TriggerMode::Toggle,
    };

    // Determine input target (mouse vs keyboard) and click type (Single/Press, Double, Hold)
    let click_type_idx = get_click_type_from_combo(ctx.controls.combo_click_type, ctx.state.input_mode);
    let click_type = match click_type_idx {
        1 => ClickType::Double,
        2 => ClickType::Hold,
        _ => ClickType::Single,
    };

    let click_target = if ctx.state.input_mode == 0 {
        let btn = match combobox_get_cursel(ctx.controls.combo_mouse_btn) {
            1 => MouseButton::Right,
            2 => MouseButton::Middle,
            _ => MouseButton::Left,
        };
        ClickTarget::Mouse(btn, click_type)
    } else {
        ClickTarget::Keyboard(ctx.state.target_key, click_type)
    };

    ctx.state.current_target = Some(click_target);

    // Determine mouse location mode (Current vs Specific coordinate)
    let location = if ctx.state.input_mode == 0 && ctx.state.location_mode == 1 {
        let x = parse_i32_or_0(&get_control_text(ctx.controls.edit_loc_x));
        let y = parse_i32_or_0(&get_control_text(ctx.controls.edit_loc_y));
        MouseLocation::Specific(x, y)
    } else {
        MouseLocation::Current
    };

    // Determine click amount (0 = infinite)
    let amount = parse_u64_or_0(&get_control_text(ctx.controls.edit_amount));

    let config = ClickerConfig {
        target: click_target,
        location,
        trigger_mode,
        hotkey: ctx.state.hotkey,
        interval,
        amount,
    };

    let signal = Arc::new(WorkerSignal::new());
    ctx.state.worker_signal = Some(signal.clone());
    ctx.state.is_running = true;

    // In HoldMouse trigger mode, monitor physical mouse button down/up via low-level hook
    if trigger_mode == TriggerMode::HoldMouse {
        let btn_to_monitor = if ctx.state.input_mode == 0 {
            combobox_get_cursel(ctx.controls.combo_mouse_btn) as u32
        } else {
            0 // Left mouse button by default for keyboard target
        };
        start_mouse_hold_monitor(btn_to_monitor, signal.clone());
    }

    // Save configuration settings
    save_current_settings(ctx);

    // Update UI
    let hotkey_name = vk_to_string(ctx.state.hotkey);
    set_control_text(ctx.controls.btn_start, &format!("STOP ({})", hotkey_name));
    set_controls_enabled(&ctx.controls, false);
    unsafe {
        InvalidateRect(hwnd, null_mut(), 1);
    }

    // Spawn background worker thread with compact 64KB stack
    let hwnd_raw = hwnd as isize;
    spawn_small_thread(move || {
        run_worker_loop(hwnd_raw, signal, config);
    });
}

fn spawn_small_thread<F>(f: F)
where
    F: FnOnce() + Send + 'static,
{
    if std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(f)
        .is_err()
    {
        // Fallback unreachable unless OS rejects explicit stack size
    }
}

fn stop_clicker(hwnd: HWND, ctx: &mut WindowContext) {
    if !ctx.state.is_running {
        return;
    }

    stop_mouse_hold_monitor();

    ctx.state.is_running = false;

    if let Some(signal) = ctx.state.worker_signal.take() {
        signal.stop();
    }

    ctx.state.current_target = None;

    // Update UI
    let hotkey_name = vk_to_string(ctx.state.hotkey);
    set_control_text(ctx.controls.btn_start, &format!("START ({})", hotkey_name));
    set_controls_enabled(&ctx.controls, true);
    update_location_state(&ctx.controls, ctx.state.input_mode, ctx.state.location_mode);
    unsafe {
        InvalidateRect(hwnd, null_mut(), 1);
    }
    trim_working_set();
}

fn toggle_clicker(hwnd: HWND, ctx: &mut WindowContext) {
    if ctx.state.is_running {
        stop_clicker(hwnd, ctx);
    } else {
        start_clicker(hwnd, ctx);
    }
}

/// Compact the Win32 process heap and flush unused pages from the working set back to the OS.
pub fn trim_working_set() {
    unsafe {
        let heap = GetProcessHeap();
        if !heap.is_null() {
            HeapCompact(heap, 0);
        }
        SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
}

unsafe fn post_boxed_msg<T>(hwnd: HWND, msg: u32, wparam: usize, payload: T) {
    let ptr = Box::into_raw(Box::new(payload));
    if PostMessageW(hwnd, msg, wparam, ptr as isize) == 0 {
        drop(Box::from_raw(ptr));
    }
}

unsafe fn show_message_box(hwnd: HWND, text: &str, title: &str, flags: u32) -> i32 {
    with_wide_null(text, |text_ptr| {
        with_wide_null(title, |title_ptr| MessageBoxW(hwnd, text_ptr, title_ptr, flags))
    })
}

fn spawn_update_check(hwnd: HWND, is_manual: bool, delay_ms: u64) {
    let hwnd_num = hwnd as usize;
    let manual_flag = usize::from(is_manual);
    spawn_small_thread(move || {
        if delay_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        }
        unsafe {
            let hwnd = hwnd_num as HWND;
            match updater::check_for_updates() {
                Ok(Some(info)) => post_boxed_msg(hwnd, WM_APP_UPDATE_AVAILABLE, manual_flag, info),
                Ok(None) => {
                    PostMessageW(hwnd, WM_APP_UPDATE_NOT_FOUND, manual_flag, 0);
                }
                Err(e) => post_boxed_msg(hwnd, WM_APP_UPDATE_CHECK_FAILED, manual_flag, e),
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        trim_working_set();
    });
}

fn trigger_update_check(hwnd: HWND, ctx: &mut WindowContext, is_manual: bool) {
    if matches!(ctx.state.update_status, UpdateStatus::Checking | UpdateStatus::Downloading) {
        return;
    }
    ctx.state.update_status = UpdateStatus::Checking;
    spawn_update_check(hwnd, is_manual, 0);
}

fn start_download_and_install(hwnd: HWND, ctx: &mut WindowContext, download_url: String) {
    save_current_settings(ctx);
    ctx.state.update_status = UpdateStatus::Downloading;
    let hwnd_num = hwnd as usize;
    spawn_small_thread(move || {
        let hwnd = hwnd_num as HWND;
        let result = updater::download_update(&download_url)
            .and_then(|temp_path| updater::apply_update(&temp_path));
        unsafe {
            match result {
                Ok(()) => {
                    PostMessageW(hwnd, WM_CLOSE, 0, 0);
                }
                Err(e) => post_boxed_msg(hwnd, WM_APP_UPDATE_FAILED, 0, e),
            }
        }
    });
}

fn cancel_active_rebind(ctx: &mut WindowContext) {
    if ctx.state.rebind_target == RebindTarget::None {
        return;
    }
    stop_rebind();
    match ctx.state.rebind_target {
        RebindTarget::Hotkey => set_control_text(ctx.controls.btn_hotkeys, "Set Hotkey"),
        RebindTarget::TargetKey => {
            let name = vk_to_string(ctx.state.target_key as u32);
            set_control_text(ctx.controls.btn_key_target, &format!("Key: {}", name));
        }
        RebindTarget::PickLocation => set_control_text(ctx.controls.btn_pick_pos, "Pick"),
        RebindTarget::None => {}
    }
    ctx.state.rebind_target = RebindTarget::None;
}

fn toggle_rebind_mode(hwnd: HWND, ctx: &mut WindowContext, target: RebindTarget) {
    if ctx.state.rebind_target == target {
        cancel_active_rebind(ctx);
        return;
    }
    cancel_active_rebind(ctx);
    ctx.state.rebind_target = target;
    match target {
        RebindTarget::Hotkey => {
            set_control_text(ctx.controls.btn_hotkeys, "Press key...");
            start_rebind(hwnd, RebindTarget::Hotkey);
        }
        RebindTarget::TargetKey => {
            set_control_text(ctx.controls.btn_key_target, "Press key...");
            start_rebind(hwnd, RebindTarget::TargetKey);
        }
        RebindTarget::PickLocation => {
            set_control_text(ctx.controls.btn_pick_pos, "Click...");
            start_pick_location(hwnd);
        }
        RebindTarget::None => {}
    }
}

fn set_timing_mode(hwnd: HWND, ctx: &mut WindowContext, mode: u32) {
    if ctx.state.is_running || ctx.state.timing_mode == mode {
        return;
    }
    ctx.state.timing_mode = mode;
    button_set_checked(ctx.controls.radio_interval, mode == 0);
    button_set_checked(ctx.controls.radio_rate, mode == 1);
    update_timing_visibility(&ctx.controls, mode);
    save_current_settings(ctx);
    unsafe {
        InvalidateRect(hwnd, null_mut(), 1);
    }
}

fn set_input_mode(hwnd: HWND, ctx: &mut WindowContext, mode: u32) {
    if ctx.state.is_running || ctx.state.input_mode == mode {
        return;
    }
    let current_type = get_click_type_from_combo(ctx.controls.combo_click_type, ctx.state.input_mode);
    let current_trigger = combobox_get_cursel(ctx.controls.combo_trigger_mode);
    ctx.state.input_mode = mode;
    button_set_checked(ctx.controls.radio_mouse, mode == 0);
    button_set_checked(ctx.controls.radio_keyboard, mode == 1);
    update_input_visibility(&ctx.controls, mode);
    update_click_type_options(ctx.controls.combo_click_type, mode, current_type);
    update_trigger_mode_options(ctx.controls.combo_trigger_mode, mode, current_trigger);
    ctx.state.trigger_mode = combobox_get_cursel(ctx.controls.combo_trigger_mode) as u32;
    update_location_state(&ctx.controls, mode, ctx.state.location_mode);
    let label = if mode == 0 {
        "Click Amount (0 = forever):"
    } else {
        "Press Amount (0 = forever):"
    };
    set_control_text(ctx.controls.lbl_amount, label);
    save_current_settings(ctx);
    unsafe {
        InvalidateRect(hwnd, null_mut(), 1);
    }
}

fn set_location_mode(hwnd: HWND, ctx: &mut WindowContext, mode: u32) {
    if ctx.state.is_running || ctx.state.input_mode != 0 || ctx.state.location_mode == mode {
        return;
    }
    ctx.state.location_mode = mode;
    button_set_checked(ctx.controls.radio_loc_current, mode == 0);
    button_set_checked(ctx.controls.radio_loc_specific, mode == 1);
    update_location_state(&ctx.controls, ctx.state.input_mode, mode);
    save_current_settings(ctx);
    unsafe {
        InvalidateRect(hwnd, null_mut(), 1);
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            let cs = &*(lparam as *const CREATESTRUCTW);
            let hinstance = cs.hInstance;

            let (font_reg, font_bold) = create_ui_fonts();
            let controls = create_controls(hwnd, hinstance, font_reg, font_bold);

            // Load saved settings (persists user choices across sessions)
            let settings_fallback;
            let settings = if !cs.lpCreateParams.is_null() {
                &*(cs.lpCreateParams as *const AppSettings)
            } else {
                settings_fallback = load_settings();
                &settings_fallback
            };

            // 1. Timing mode and inputs
            if settings.timing_mode == 0 {
                button_set_checked(controls.radio_interval, true);
                button_set_checked(controls.radio_rate, false);
            } else {
                button_set_checked(controls.radio_interval, false);
                button_set_checked(controls.radio_rate, true);
            }
            set_control_text(controls.edit_hr, &settings.hr);
            set_control_text(controls.edit_min, &settings.min);
            set_control_text(controls.edit_sec, &settings.sec);
            set_control_text(controls.edit_ms, &settings.ms);
            set_control_text(controls.edit_rate, &settings.rate);
            combobox_set_cursel(controls.combo_rate_unit, settings.rate_unit);

            // 2. Input mode and targets
            if settings.input_mode == 0 {
                button_set_checked(controls.radio_mouse, true);
                button_set_checked(controls.radio_keyboard, false);
            } else {
                button_set_checked(controls.radio_mouse, false);
                button_set_checked(controls.radio_keyboard, true);
            }
            combobox_set_cursel(controls.combo_mouse_btn, settings.mouse_btn);
            update_click_type_options(controls.combo_click_type, settings.input_mode, settings.click_type);

            let key_name = vk_to_string(settings.target_key as u32);
            set_control_text(controls.btn_key_target, &format!("Key: {}", key_name));

            // 3. Location mode and coordinates
            if settings.location_mode == 0 {
                button_set_checked(controls.radio_loc_current, true);
                button_set_checked(controls.radio_loc_specific, false);
            } else {
                button_set_checked(controls.radio_loc_current, false);
                button_set_checked(controls.radio_loc_specific, true);
            }
            set_control_text(controls.edit_loc_x, &settings.target_x.to_string());
            set_control_text(controls.edit_loc_y, &settings.target_y.to_string());

            // 4. Click amount
            set_control_text(controls.edit_amount, &settings.click_amount);
            let amount_label = if settings.input_mode == 0 {
                "Click Amount (0 = forever):"
            } else {
                "Press Amount (0 = forever):"
            };
            set_control_text(controls.lbl_amount, amount_label);

            // 5. Start/Stop hotkey
            let hotkey_name = vk_to_string(settings.hotkey);
            set_control_text(controls.btn_start, &format!("START ({})", hotkey_name));

            update_trigger_mode_options(controls.combo_trigger_mode, settings.input_mode, settings.trigger_mode as usize);

            update_timing_visibility(&controls, settings.timing_mode);
            update_input_visibility(&controls, settings.input_mode);
            update_location_state(&controls, settings.input_mode, settings.location_mode);

            // Register remembered Start/Stop hotkey
            register_start_stop_hotkey(hwnd, settings.hotkey);

            let mut state = AppState::default();
            state.hotkey = settings.hotkey;
            state.target_key = settings.target_key;
            state.trigger_mode = settings.trigger_mode;
            state.timing_mode = settings.timing_mode;
            state.input_mode = settings.input_mode;
            state.location_mode = settings.location_mode;
            state.target_x = settings.target_x;
            state.target_y = settings.target_y;
            state.pos_x = settings.pos_x;
            state.pos_y = settings.pos_y;

            let ctx = Box::new(WindowContext {
                hwnd,
                state,
                controls,
                theme: ThemeBrushes::new(),
                font_reg,
                font_bold,
                last_saved_settings: Some(settings.clone()),
            });

            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(ctx) as isize);

            let mut rc: RECT = std::mem::zeroed();
            GetWindowRect(hwnd, &mut rc);
            update_window_bounds(rc.left, rc.top, rc.right, rc.bottom);

            0
        }

        WM_MOVE => {
            let ctx_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowContext;
            if !ctx_ptr.is_null() {
                let ctx = &mut *ctx_ptr;
                if IsIconic(hwnd) == 0 {
                    let mut rc: RECT = std::mem::zeroed();
                    GetWindowRect(hwnd, &mut rc);
                    ctx.state.pos_x = rc.left;
                    ctx.state.pos_y = rc.top;
                    update_window_bounds(rc.left, rc.top, rc.right, rc.bottom);
                }
            }
            0
        }

        WM_SIZE => {
            if wparam == SIZE_MINIMIZED as usize {
                trim_working_set();
            }
            0
        }

        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }

        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);
            let ctx_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowContext;
            if !ctx_ptr.is_null() {
                let ctx = &*ctx_ptr;
                // Paint elevated dark cards and rounded edit input pills
                draw_ui_chrome(hdc, &ctx.controls, &ctx.state);
            }
            EndPaint(hwnd, &mut ps);
            0
        }

        WM_DRAWITEM => {
            let dis = &*(lparam as *const DRAWITEMSTRUCT);
            let ctx_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowContext;
            if !ctx_ptr.is_null() {
                let ctx = &*ctx_ptr;
                draw_custom_button(dis, &ctx.state, ctx.font_bold, ctx.font_reg);
            }
            1
        }

        WM_COMMAND => {
            let ctrl_id = (wparam & 0xFFFF) as usize;
            let notification = (wparam >> 16) as u16;
            let ctx_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowContext;
            if ctx_ptr.is_null() {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            }
            let ctx = &mut *ctx_ptr;

            const BN_CLICKED_U16: u16 = 0;

            match ctrl_id {
                ID_BTN_START => {
                    if notification == BN_CLICKED_U16 {
                        toggle_clicker(hwnd, ctx);
                    }
                }

                ID_BTN_HOTKEYS => {
                    if notification == BN_CLICKED_U16 {
                        toggle_rebind_mode(hwnd, ctx, RebindTarget::Hotkey);
                    }
                }

                ID_BTN_KEY_TARGET => {
                    if notification == BN_CLICKED_U16 {
                        toggle_rebind_mode(hwnd, ctx, RebindTarget::TargetKey);
                    }
                }

                ID_BTN_PICK_POS => {
                    if notification == BN_CLICKED_U16 {
                        toggle_rebind_mode(hwnd, ctx, RebindTarget::PickLocation);
                    }
                }

                ID_RADIO_INTERVAL | ID_LBL_RADIO_INTERVAL => {
                    if notification == BN_CLICKED_U16 {
                        set_timing_mode(hwnd, ctx, 0);
                    }
                }

                ID_RADIO_RATE | ID_LBL_RADIO_RATE => {
                    if notification == BN_CLICKED_U16 {
                        set_timing_mode(hwnd, ctx, 1);
                    }
                }

                ID_RADIO_MOUSE | ID_LBL_RADIO_MOUSE => {
                    if notification == BN_CLICKED_U16 {
                        set_input_mode(hwnd, ctx, 0);
                    }
                }

                ID_RADIO_KEYBOARD | ID_LBL_RADIO_KEYBOARD => {
                    if notification == BN_CLICKED_U16 {
                        set_input_mode(hwnd, ctx, 1);
                    }
                }

                ID_RADIO_LOC_CURRENT | ID_LBL_RADIO_LOC_CURRENT => {
                    if notification == BN_CLICKED_U16 {
                        set_location_mode(hwnd, ctx, 0);
                    }
                }

                ID_RADIO_LOC_SPECIFIC | ID_LBL_RADIO_LOC_SPECIFIC => {
                    if notification == BN_CLICKED_U16 {
                        set_location_mode(hwnd, ctx, 1);
                    }
                }

                ID_COMBO_RATE_UNIT | ID_COMBO_MOUSE_BTN | ID_COMBO_CLICK_TYPE | ID_COMBO_TRIGGER_MODE => {
                    if notification == CBN_SELCHANGE as u16 {
                        ctx.state.trigger_mode = combobox_get_cursel(ctx.controls.combo_trigger_mode) as u32;
                        save_current_settings(ctx);
                    }
                }

                ID_EDIT_HR | ID_EDIT_MIN | ID_EDIT_SEC | ID_EDIT_MS | ID_EDIT_RATE | ID_EDIT_AMOUNT | ID_EDIT_LOC_X | ID_EDIT_LOC_Y => {
                    if notification == EN_KILLFOCUS as u16 {
                        save_current_settings(ctx);
                    }
                }

                _ => {}
            }
            0
        }

        WM_HOTKEY => {
            let hotkey_id = wparam as i32;
            if hotkey_id == HOTKEY_ID_START_STOP {
                let ctx_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowContext;
                if !ctx_ptr.is_null() {
                    let ctx = &mut *ctx_ptr;
                    let trigger_mode = combobox_get_cursel(ctx.controls.combo_trigger_mode);
                    if trigger_mode == 1 {
                        // Hold mode (hotkey): start if not already running
                        if !ctx.state.is_running {
                            start_clicker(hwnd, ctx);
                        }
                    } else {
                        // Toggle mode or HoldMouse mode: toggle start/stop
                        toggle_clicker(hwnd, ctx);
                    }
                }
            }
            0
        }

        WM_APP_STOPPED => {
            // Click count limit was reached or hold released
            let ctx_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowContext;
            if !ctx_ptr.is_null() {
                let ctx = &mut *ctx_ptr;
                stop_clicker(hwnd, ctx);
            }
            0
        }

        WM_APP_KEY_CAPTURED => {
            let target_code = wparam as u32;
            let vk = lparam as u32;

            let ctx_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowContext;
            if !ctx_ptr.is_null() {
                let ctx = &mut *ctx_ptr;
                stop_rebind();
                ctx.state.rebind_target = RebindTarget::None;

                if target_code == RebindTarget::Hotkey as u32 {
                    if vk != VK_ESCAPE as u32 {
                        // Conflict Prevention: Start/Stop hotkey and Target Key must NOT be the same!
                        if vk == ctx.state.target_key as u32 {
                            MessageBeep(0xFFFFFFFF);
                            set_control_text(ctx.controls.btn_hotkeys, "Key in use!");
                            // Keep listening for a different key
                            ctx.state.rebind_target = RebindTarget::Hotkey;
                            start_rebind(hwnd, RebindTarget::Hotkey);
                            return 0;
                        }

                        // Rebind global hotkey
                        unregister_start_stop_hotkey(hwnd);
                        register_start_stop_hotkey(hwnd, vk);
                        ctx.state.hotkey = vk;

                        let hotkey_name = vk_to_string(vk);
                        let start_label = if ctx.state.is_running {
                            format!("STOP ({})", hotkey_name)
                        } else {
                            format!("START ({})", hotkey_name)
                        };
                        set_control_text(ctx.controls.btn_start, &start_label);
                        save_current_settings(ctx);
                    }
                    set_control_text(ctx.controls.btn_hotkeys, "Set Hotkey");
                } else if target_code == RebindTarget::TargetKey as u32 {
                    if vk != VK_ESCAPE as u32 {
                        // Conflict Prevention: Target Key and Start/Stop hotkey must NOT be the same!
                        if vk == ctx.state.hotkey {
                            MessageBeep(0xFFFFFFFF);
                            set_control_text(ctx.controls.btn_key_target, "Key in use!");
                            // Keep listening for a different key
                            ctx.state.rebind_target = RebindTarget::TargetKey;
                            start_rebind(hwnd, RebindTarget::TargetKey);
                            return 0;
                        }

                        ctx.state.target_key = vk as u16;
                        save_current_settings(ctx);
                    }
                    let key_name = vk_to_string(ctx.state.target_key as u32);
                    set_control_text(ctx.controls.btn_key_target, &format!("Key: {}", key_name));
                } else if target_code == RebindTarget::PickLocation as u32 {
                    set_control_text(ctx.controls.btn_pick_pos, "Pick");
                    InvalidateRect(hwnd, null_mut(), 1);
                }
            }
            0
        }

        WM_APP_LOCATION_CAPTURED => {
            let x = wparam as i32;
            let y = lparam as i32;

            let ctx_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowContext;
            if !ctx_ptr.is_null() {
                let ctx = &mut *ctx_ptr;
                stop_rebind();
                ctx.state.rebind_target = RebindTarget::None;
                set_control_text(ctx.controls.btn_pick_pos, "Pick");

                ctx.state.target_x = x;
                ctx.state.target_y = y;
                set_control_text(ctx.controls.edit_loc_x, &x.to_string());
                set_control_text(ctx.controls.edit_loc_y, &y.to_string());

                // Auto-switch to Specific location mode upon picking a location
                ctx.state.location_mode = 1;
                button_set_checked(ctx.controls.radio_loc_current, false);
                button_set_checked(ctx.controls.radio_loc_specific, true);
                update_location_state(&ctx.controls, ctx.state.input_mode, 1);

                save_current_settings(ctx);
                InvalidateRect(hwnd, null_mut(), 1);
            }
            0
        }

        WM_CTLCOLORSTATIC => {
            let hdc = wparam as HDC;
            let ctrl_hwnd = lparam as HWND;
            let ctx_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowContext;
            if !ctx_ptr.is_null() {
                let ctx = &*ctx_ptr;
                let is_edit = [
                    ctx.controls.edit_hr,
                    ctx.controls.edit_min,
                    ctx.controls.edit_sec,
                    ctx.controls.edit_ms,
                    ctx.controls.edit_rate,
                    ctx.controls.edit_amount,
                    ctx.controls.edit_loc_x,
                    ctx.controls.edit_loc_y,
                ]
                .contains(&ctrl_hwnd);

                if is_edit {
                    SetBkMode(hdc, 2 /* OPAQUE */);
                    SetBkColor(hdc, COLOR_SURFACE_DIS);
                    SetTextColor(hdc, COLOR_TEXT_MUTED);
                    return ctx.theme.disabled_edit_brush as LRESULT;
                }

                let is_secondary_unit = [
                    ctx.controls.lbl_hr,
                    ctx.controls.lbl_min,
                    ctx.controls.lbl_sec,
                    ctx.controls.lbl_ms,
                    ctx.controls.lbl_loc_x,
                    ctx.controls.lbl_loc_y,
                ]
                .contains(&ctrl_hwnd);

                // Dim inactive or running labels with clean COLOR_TEXT_MUTED font (no 3D emboss)
                let is_dimmed = ctx.state.is_running
                    || is_secondary_unit
                    || (ctx.state.input_mode == 1
                        && [
                            ctx.controls.lbl_location,
                            ctx.controls.lbl_radio_loc_current,
                            ctx.controls.lbl_radio_loc_specific,
                        ]
                        .contains(&ctrl_hwnd));

                let text_color = if is_dimmed {
                    COLOR_TEXT_MUTED
                } else {
                    COLOR_TEXT_PRIMARY
                };

                SetBkMode(hdc, 1 /* TRANSPARENT */);
                SetBkColor(hdc, COLOR_CARD);
                SetTextColor(hdc, text_color);
                return ctx.theme.card_brush as LRESULT;
            }
            GetSysColorBrush(15) as LRESULT
        }

        WM_CTLCOLOREDIT => {
            let hdc = wparam as HDC;
            let ctx_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowContext;
            if !ctx_ptr.is_null() {
                let ctx = &*ctx_ptr;
                SetBkColor(hdc, COLOR_SURFACE);
                SetTextColor(hdc, rgb(255, 255, 255));
                return ctx.theme.edit_brush as LRESULT;
            }
            GetSysColorBrush(15) as LRESULT
        }

        WM_CTLCOLORLISTBOX => {
            let hdc = wparam as HDC;
            let ctx_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowContext;
            if !ctx_ptr.is_null() {
                let ctx = &*ctx_ptr;
                SetBkColor(hdc, COLOR_SURFACE);
                SetTextColor(hdc, rgb(255, 255, 255));
                return ctx.theme.edit_brush as LRESULT;
            }
            GetSysColorBrush(15) as LRESULT
        }

        WM_ACTIVATE => {
            let active_type = (wparam & 0xFFFF) as u16;
            if active_type == WA_INACTIVE as u16 {
                // If window loses focus while listening for Hotkey or TargetKey, cancel rebind to avoid trapped hooks
                let ctx_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowContext;
                if !ctx_ptr.is_null() {
                    let ctx = &mut *ctx_ptr;
                    if matches!(ctx.state.rebind_target, RebindTarget::Hotkey | RebindTarget::TargetKey) {
                        cancel_active_rebind(ctx);
                    }
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        WM_DESTROY => {
            let ctx_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowContext;
            if !ctx_ptr.is_null() {
                let mut ctx = Box::from_raw(ctx_ptr);
                if ctx.state.is_running {
                    stop_clicker(hwnd, &mut ctx);
                }
                save_current_settings(&mut ctx);
                stop_rebind();
                unregister_start_stop_hotkey(hwnd);
                ctx.theme.destroy();
                DeleteObject(ctx.font_reg);
                DeleteObject(ctx.font_bold);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            PostQuitMessage(0);
            0
        }

        WM_SYSCOMMAND => {
            let cmd = (wparam & 0xFFF0) as u32;
            if cmd == IDM_CHECK_UPDATES {
                let ctx_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowContext;
                if !ctx_ptr.is_null() {
                    let ctx = &mut *ctx_ptr;
                    trigger_update_check(hwnd, ctx, true);
                }
                0
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }

        WM_APP_UPDATE_AVAILABLE => {
            let _is_manual = wparam == 1;
            let update = Box::from_raw(lparam as *mut UpdateInfo);
            let tag = update.tag.clone();
            let download_url = update.download_url.clone();
            let ctx_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowContext;
            if !ctx_ptr.is_null() {
                let ctx = &mut *ctx_ptr;
                ctx.state.update_status = UpdateStatus::Available(update);
            }

            let msg = format!(
                "A new version of Clickr is available!\n\n\
                 Current version: v{}\n\
                 Latest version: {}\n\n\
                 Would you like to download and install the update now?",
                updater::CURRENT_VERSION,
                tag
            );
            let res = show_message_box(
                hwnd,
                &msg,
                "Clickr Update Available",
                MB_YESNO | MB_ICONINFORMATION | MB_DEFBUTTON1,
            );
            if res == IDYES && !ctx_ptr.is_null() {
                let ctx = &mut *ctx_ptr;
                start_download_and_install(hwnd, ctx, download_url);
            }
            0
        }

        WM_APP_UPDATE_NOT_FOUND => {
            let is_manual = wparam == 1;
            let ctx_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowContext;
            if !ctx_ptr.is_null() {
                let ctx = &mut *ctx_ptr;
                ctx.state.update_status = UpdateStatus::UpToDate;
            }
            if is_manual {
                let msg = format!(
                    "You are already using the latest version of Clickr (v{}).",
                    updater::CURRENT_VERSION
                );
                show_message_box(hwnd, &msg, "Up to Date", MB_OK | MB_ICONINFORMATION);
            }
            0
        }

        WM_APP_UPDATE_CHECK_FAILED => {
            let is_manual = wparam == 1;
            let err_msg = Box::from_raw(lparam as *mut String);
            let ctx_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowContext;
            if !ctx_ptr.is_null() {
                let ctx = &mut *ctx_ptr;
                ctx.state.update_status = UpdateStatus::Failed((*err_msg).clone());
            }
            if is_manual {
                let msg = format!("Failed to check for updates:\n\n{}", err_msg);
                show_message_box(hwnd, &msg, "Update Check Failed", MB_OK | MB_ICONWARNING);
            }
            0
        }

        WM_APP_UPDATE_FAILED => {
            let err_msg = Box::from_raw(lparam as *mut String);
            let ctx_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowContext;
            if !ctx_ptr.is_null() {
                let ctx = &mut *ctx_ptr;
                ctx.state.update_status = UpdateStatus::Failed((*err_msg).clone());
            }
            let msg = format!("Failed to install update:\n\n{}", err_msg);
            show_message_box(hwnd, &msg, "Update Error", MB_OK | MB_ICONERROR);
            0
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn main() {
    unsafe {
        // Clean up previous backup binary if an update was just installed
        updater::cleanup_old_version();

        // Initialize modern Common Controls v6
        let mut icc = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_STANDARD_CLASSES | ICC_WIN95_CLASSES,
        };
        InitCommonControlsEx(&mut icc);

        let hinstance = GetModuleHandleW(null_mut());
        let class_name = to_wide_null("ClickrWindowClass");

        // Load custom embedded modern application icon
        let icon_big = LoadImageW(
            hinstance,
            1 as *const u16,
            IMAGE_ICON,
            32,
            32,
            LR_SHARED | LR_DEFAULTCOLOR,
        ) as HICON;

        let icon_small = LoadImageW(
            hinstance,
            1 as *const u16,
            IMAGE_ICON,
            16,
            16,
            LR_SHARED | LR_DEFAULTCOLOR,
        ) as HICON;

        let app_icon = if icon_big != null_mut() {
            icon_big
        } else {
            LoadIconW(null_mut(), IDI_APPLICATION)
        };

        let bg_brush = CreateSolidBrush(COLOR_BG);

        let wnd_class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: app_icon,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: bg_brush,
            lpszMenuName: null_mut(),
            lpszClassName: class_name.as_ptr(),
        };

        RegisterClassW(&wnd_class);

        // Fixed-size, compact window with refined card layout
        let window_style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN;
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: UI_CLIENT_WIDTH,
            bottom: UI_CLIENT_HEIGHT,
        };
        AdjustWindowRectEx(&mut rect, window_style, 0, 0);

        let win_width = rect.right - rect.left;
        let win_height = rect.bottom - rect.top;

        let settings = load_settings();

        // Calculate initial window position (restores remembered position or centers on primary display)
        let screen_w = GetSystemMetrics(SM_CXSCREEN);
        let screen_h = GetSystemMetrics(SM_CYSCREEN);
        let mut pos_x = (screen_w - win_width) / 2;
        let mut pos_y = (screen_h - win_height) / 2;

        if settings.pos_x != -1 && settings.pos_y != -1 {
            // Verify stored position is on an active display monitor
            let pt = POINT {
                x: settings.pos_x + 40,
                y: settings.pos_y + 40,
            };
            let hmon = MonitorFromPoint(pt, MONITOR_DEFAULTTONULL);
            if hmon != null_mut() {
                pos_x = settings.pos_x;
                pos_y = settings.pos_y;
            }
        }

        let hwnd = with_wide_null("Clickr", |title_ptr| {
            CreateWindowExW(
                0,
                class_name.as_ptr(),
                title_ptr,
                window_style,
                pos_x,
                pos_y,
                win_width,
                win_height,
                null_mut(),
                null_mut(),
                hinstance,
                &settings as *const AppSettings as *const std::ffi::c_void,
            )
        });

        // Set modern application icon explicitly on window instance
        if icon_big != null_mut() {
            SendMessageW(hwnd, WM_SETICON, ICON_BIG as WPARAM, icon_big as LPARAM);
        }
        if icon_small != null_mut() {
            SendMessageW(hwnd, WM_SETICON, ICON_SMALL as WPARAM, icon_small as LPARAM);
        }

        // Add Check for Updates to window system menu (Alt+Space or title bar right-click)
        let sys_menu = GetSystemMenu(hwnd, 0);
        if sys_menu != null_mut() {
            AppendMenuW(sys_menu, MF_SEPARATOR, 0, null());
            with_wide_null("Check for Updates...", |label_ptr| {
                AppendMenuW(sys_menu, MF_STRING, IDM_CHECK_UPDATES as usize, label_ptr);
            });
        }

        // Enable Windows 10/11 native immersive dark mode title bar
        let dark: i32 = 1;
        DwmSetWindowAttribute(
            hwnd,
            20, // DWMWA_USE_IMMERSIVE_DARK_MODE (Windows 10 20H1+ and Windows 11)
            &dark as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        );
        DwmSetWindowAttribute(
            hwnd,
            19, // DWMWA_USE_IMMERSIVE_DARK_MODE (Windows 10 1809/1903 fallback)
            &dark as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        );

        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);
        trim_working_set();

        // Background update check on startup (non-blocking)
        spawn_update_check(hwnd, false, 800);

        let mut initial_rc: RECT = std::mem::zeroed();
        GetWindowRect(hwnd, &mut initial_rc);
        update_window_bounds(initial_rc.left, initial_rc.top, initial_rc.right, initial_rc.bottom);

        // Native Windows message loop: 0.0% CPU and 0.0% GPU when idle
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            if IsDialogMessageW(hwnd, &mut msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        DeleteObject(bg_brush);
    }
}

