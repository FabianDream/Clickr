use std::ptr::null_mut;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::Controls::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_F6, VK_SPACE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::clicker::{ClickTarget, WorkerSignal};
use crate::hotkeys::RebindTarget;

#[link(name = "uxtheme")]
extern "system" {
    fn SetWindowTheme(
        hwnd: HWND,
        pszSubAppName: *const u16,
        pszSubIdList: *const u16,
    ) -> i32;
}

pub const ID_BTN_START: usize = 1001;
pub const ID_BTN_HOTKEYS: usize = 1002;
pub const ID_BTN_HOTKEY: usize = 1002;

pub const ID_RADIO_INTERVAL: usize = 1003;
pub const ID_RADIO_RATE: usize = 1004;

pub const ID_EDIT_HR: usize = 1005;
pub const ID_EDIT_MIN: usize = 1006;
pub const ID_EDIT_SEC: usize = 1007;
pub const ID_EDIT_MS: usize = 1008;

pub const ID_EDIT_RATE: usize = 1009;
pub const ID_COMBO_RATE_UNIT: usize = 1010;

pub const ID_RADIO_MOUSE: usize = 1011;
pub const ID_RADIO_KEYBOARD: usize = 1012;

pub const ID_COMBO_MOUSE_BTN: usize = 1013;
pub const ID_BTN_KEY_TARGET: usize = 1014;

pub const ID_EDIT_AMOUNT: usize = 1015;
pub const ID_LBL_RADIO_INTERVAL: usize = 1016;
pub const ID_LBL_RADIO_RATE: usize = 1017;
pub const ID_LBL_RADIO_MOUSE: usize = 1018;
pub const ID_LBL_RADIO_KEYBOARD: usize = 1019;

// Row 3 - Location Controls
pub const ID_RADIO_LOC_CURRENT: usize = 1020;
pub const ID_LBL_RADIO_LOC_CURRENT: usize = 1021;
pub const ID_RADIO_LOC_SPECIFIC: usize = 1022;
pub const ID_LBL_RADIO_LOC_SPECIFIC: usize = 1023;
pub const ID_EDIT_LOC_X: usize = 1024;
pub const ID_EDIT_LOC_Y: usize = 1025;
pub const ID_BTN_PICK_POS: usize = 1026;

// Row 2 - Click Type Control (Single, Double, Hold)
pub const ID_COMBO_CLICK_TYPE: usize = 1027;

// Row 0 - Trigger Mode Control (Toggle vs Hold)
pub const ID_COMBO_TRIGGER_MODE: usize = 1028;

pub const BST_CHECKED_VAL: usize = 1;
pub const BST_UNCHECKED_VAL: usize = 0;
pub const SS_CENTERIMAGE_STYLE: u32 = 0x0200;
pub const SS_NOTIFY_STYLE: u32 = 0x0100;

// Dark Palette Colors
pub const COLOR_BG: u32 = rgb(18, 18, 22);            // Deep Canvas (#121216)
pub const COLOR_CARD: u32 = rgb(27, 27, 32);          // Elevated Card (#1B1B20)
pub const COLOR_CARD_BORDER: u32 = rgb(46, 46, 54);   // Subtle Card Border (#2E2E36)
pub const COLOR_SURFACE: u32 = rgb(38, 38, 45);       // Input Surface (#26262D)
pub const COLOR_SURFACE_DIS: u32 = rgb(23, 23, 28);   // Disabled Input Surface
pub const COLOR_TEXT_PRIMARY: u32 = rgb(244, 244, 247);
pub const COLOR_TEXT_MUTED: u32 = rgb(156, 156, 168);
pub const COLOR_BORDER_SUBTLE: u32 = rgb(64, 64, 74);
pub const COLOR_BORDER_DIS: u32 = rgb(42, 42, 50);

pub const UI_CLIENT_WIDTH: i32 = 520;
pub const UI_CLIENT_HEIGHT: i32 = 284;

const BS_OWNERDRAW_U32: u32 = BS_OWNERDRAW as u32;
const BS_AUTORADIOBUTTON_U32: u32 = BS_AUTORADIOBUTTON as u32;
const ES_CENTER_U32: u32 = ES_CENTER as u32;
const ES_NUMBER_U32: u32 = ES_NUMBER as u32;
const CBS_DROPDOWNLIST_U32: u32 = CBS_DROPDOWNLIST as u32;

const TTM_SETMAXTIPWIDTH: u32 = WM_USER + 24;
const TTM_ADDTOOLW: u32 = WM_USER + 50;
const TTF_IDISHWND: u32 = 0x0001;
const TTF_SUBCLASS: u32 = 0x0010;
const TTS_ALWAYSTIP: u32 = 0x01;
const TTS_NOPREFIX: u32 = 0x02;

pub const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

#[inline]
pub fn with_wide_null<R>(s: &str, f: impl FnOnce(*const u16) -> R) -> R {
    let mut stack_buf = [0u16; 128];
    let mut len = 0usize;
    for unit in s.encode_utf16() {
        if len + 1 >= stack_buf.len() {
            let wide: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
            return f(wide.as_ptr());
        }
        stack_buf[len] = unit;
        len += 1;
    }
    stack_buf[len] = 0;
    f(stack_buf.as_ptr())
}

pub fn to_wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub struct AppState {
    pub is_running: bool,
    pub hotkey: u32,
    pub target_key: u16,
    pub trigger_mode: u32, // 0 = Toggle, 1 = Hold, 2 = HoldMouse
    pub current_target: Option<ClickTarget>,
    pub rebind_target: RebindTarget,
    pub worker_signal: Option<Arc<WorkerSignal>>,
    pub timing_mode: u32, // 0 = Interval, 1 = Rate
    pub input_mode: u32,  // 0 = Mouse, 1 = Keyboard
    pub location_mode: u32, // 0 = Current, 1 = Specific
    pub target_x: i32,
    pub target_y: i32,
    pub pos_x: i32,
    pub pos_y: i32,
    pub update_status: crate::updater::UpdateStatus,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            is_running: false,
            hotkey: VK_F6 as u32,
            target_key: VK_SPACE,
            trigger_mode: 0, // Default: Toggle
            current_target: None,
            rebind_target: RebindTarget::None,
            worker_signal: None,
            timing_mode: 1, // Default: Click Rate (requested by user)
            input_mode: 0,  // Default: Mouse (requested by user)
            location_mode: 0, // Default: Current Location
            target_x: 0,
            target_y: 0,
            pos_x: -1,
            pos_y: -1,
            update_status: crate::updater::UpdateStatus::Idle,
        }
    }
}

pub struct ThemeBrushes {
    pub bg_brush: HBRUSH,
    pub card_brush: HBRUSH,
    pub edit_brush: HBRUSH,
    pub disabled_edit_brush: HBRUSH,
}

impl ThemeBrushes {
    pub unsafe fn new() -> Self {
        Self {
            bg_brush: CreateSolidBrush(COLOR_BG),
            card_brush: CreateSolidBrush(COLOR_CARD),
            edit_brush: CreateSolidBrush(COLOR_SURFACE),
            disabled_edit_brush: CreateSolidBrush(COLOR_SURFACE_DIS),
        }
    }

    pub unsafe fn destroy(&self) {
        DeleteObject(self.bg_brush);
        DeleteObject(self.card_brush);
        DeleteObject(self.edit_brush);
        DeleteObject(self.disabled_edit_brush);
    }
}

pub struct Controls {
    pub btn_start: HWND,
    pub btn_hotkeys: HWND,
    pub combo_trigger_mode: HWND,

    // Row 1 - Timing
    pub lbl_timing: HWND,
    pub radio_interval: HWND,
    pub lbl_radio_interval: HWND,
    pub radio_rate: HWND,
    pub lbl_radio_rate: HWND,

    pub edit_hr: HWND,
    pub lbl_hr: HWND,
    pub edit_min: HWND,
    pub lbl_min: HWND,
    pub edit_sec: HWND,
    pub lbl_sec: HWND,
    pub edit_ms: HWND,
    pub lbl_ms: HWND,

    pub edit_rate: HWND,
    pub combo_rate_unit: HWND,

    // Row 2 - Buttons
    pub lbl_buttons: HWND,
    pub radio_mouse: HWND,
    pub lbl_radio_mouse: HWND,
    pub radio_keyboard: HWND,
    pub lbl_radio_keyboard: HWND,
    pub combo_mouse_btn: HWND,
    pub combo_click_type: HWND,
    pub btn_key_target: HWND,

    // Row 3 - Location
    pub lbl_location: HWND,
    pub radio_loc_current: HWND,
    pub lbl_radio_loc_current: HWND,
    pub radio_loc_specific: HWND,
    pub lbl_radio_loc_specific: HWND,
    pub lbl_loc_x: HWND,
    pub edit_loc_x: HWND,
    pub lbl_loc_y: HWND,
    pub edit_loc_y: HWND,
    pub btn_pick_pos: HWND,

    // Row 4 - Amount
    pub lbl_amount: HWND,
    pub edit_amount: HWND,
}

pub fn get_control_text(hwnd: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let needed = (len + 1) as usize;
        if needed <= 64 {
            let mut buf = [0u16; 64];
            let copied = GetWindowTextW(hwnd, buf.as_mut_ptr(), 64);
            if copied > 0 {
                String::from_utf16_lossy(&buf[..copied as usize])
            } else {
                String::new()
            }
        } else {
            let mut buf = vec![0u16; needed];
            let copied = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
            if copied > 0 {
                String::from_utf16_lossy(&buf[..copied as usize])
            } else {
                String::new()
            }
        }
    }
}

pub fn set_control_text(hwnd: HWND, text: &str) {
    with_wide_null(text, |ptr| unsafe {
        SetWindowTextW(hwnd, ptr);
        InvalidateRect(hwnd, null_mut(), 1);
    });
}

pub fn button_set_checked(hwnd: HWND, checked: bool) {
    let val = if checked {
        BST_CHECKED_VAL
    } else {
        BST_UNCHECKED_VAL
    };
    unsafe {
        SendMessageW(hwnd, BM_SETCHECK, val, 0);
    }
}

pub fn button_is_checked(hwnd: HWND) -> bool {
    unsafe {
        let res = SendMessageW(hwnd, BM_GETCHECK, 0, 0);
        res == BST_CHECKED_VAL as isize
    }
}

pub fn combobox_add_string(hwnd: HWND, text: &str) {
    with_wide_null(text, |ptr| unsafe {
        SendMessageW(hwnd, CB_ADDSTRING, 0, ptr as LPARAM);
    });
}

pub fn combobox_set_cursel(hwnd: HWND, index: usize) {
    unsafe {
        SendMessageW(hwnd, CB_SETCURSEL, index, 0);
    }
}

pub fn combobox_get_cursel(hwnd: HWND) -> usize {
    unsafe {
        let res = SendMessageW(hwnd, CB_GETCURSEL, 0, 0);
        if res < 0 {
            0
        } else {
            res as usize
        }
    }
}

/// Dynamically update click/action type options:
/// - Mouse mode: "Single", "Double", "Hold"
/// - Keyboard mode: "Press", "Hold" (double click does not exist on keyboard)
pub fn update_click_type_options(combo: HWND, input_mode: u32, current_click_type: usize) {
    unsafe {
        SendMessageW(combo, CB_RESETCONTENT, 0, 0);
        if input_mode == 0 {
            // Mouse mode: Single, Double, Hold
            combobox_add_string(combo, "Single");
            combobox_add_string(combo, "Double");
            combobox_add_string(combo, "Hold");
            let sel = current_click_type.min(2);
            combobox_set_cursel(combo, sel);
        } else {
            // Keyboard mode: Press, Hold
            combobox_add_string(combo, "Press");
            combobox_add_string(combo, "Hold");
            let sel = if current_click_type == 2 { 1 } else { 0 };
            combobox_set_cursel(combo, sel);
        }
    }
}

/// Map combobox selection to click_type:
/// - Mouse: 0 = Single, 1 = Double, 2 = Hold
/// - Keyboard: 0 = Press (maps to 0), 1 = Hold (maps to 2)
pub fn get_click_type_from_combo(combo: HWND, input_mode: u32) -> usize {
    let cursel = combobox_get_cursel(combo);
    if input_mode == 0 {
        cursel.min(2)
    } else {
        if cursel == 1 { 2 } else { 0 }
    }
}

/// Dynamically update trigger mode options:
/// - Mouse mode: "Toggle (Hotkey)", "Hold (Hotkey)", "Hold Mouse Click"
/// - Keyboard mode: "Toggle (Hotkey)", "Hold (Hotkey)" ("Hold Mouse Click" does not apply to keyboard)
pub fn update_trigger_mode_options(combo: HWND, input_mode: u32, current_trigger_mode: usize) {
    unsafe {
        SendMessageW(combo, CB_RESETCONTENT, 0, 0);
        combobox_add_string(combo, "Toggle (Hotkey)");
        combobox_add_string(combo, "Hold (Hotkey)");
        if input_mode == 0 {
            combobox_add_string(combo, "Hold Mouse Click");
            let sel = current_trigger_mode.min(2);
            combobox_set_cursel(combo, sel);
        } else {
            let sel = current_trigger_mode.min(1);
            combobox_set_cursel(combo, sel);
        }
    }
}

pub unsafe fn create_tooltip(parent: HWND, hinstance: HINSTANCE) -> HWND {
    with_wide_null("tooltips_class32", |cls| {
        let hwnd_tip = CreateWindowExW(
            WS_EX_TOPMOST,
            cls,
            null_mut(),
            WS_POPUP | TTS_NOPREFIX | TTS_ALWAYSTIP,
            0, 0, 0, 0,
            parent,
            null_mut(),
            hinstance,
            null_mut(),
        );
        SendMessageW(hwnd_tip, TTM_SETMAXTIPWIDTH, 0, 320);
        hwnd_tip
    })
}

pub unsafe fn add_tooltip(hwnd_tip: HWND, parent: HWND, ctrl_hwnd: HWND, text: &str) {
    with_wide_null(text, |ptr| {
        let mut ti: TTTOOLINFOW = std::mem::zeroed();
        ti.cbSize = std::mem::size_of::<TTTOOLINFOW>() as u32;
        ti.uFlags = TTF_SUBCLASS | TTF_IDISHWND;
        ti.hwnd = parent;
        ti.uId = ctrl_hwnd as usize;
        ti.lpszText = ptr as *mut u16;
        SendMessageW(hwnd_tip, TTM_ADDTOOLW, 0, &ti as *const _ as LPARAM);
    });
}

pub unsafe fn create_ui_fonts() -> (HFONT, HFONT) {
    with_wide_null("Segoe UI", |font_ptr| {
        let make_font = |height: i32, weight: u32| {
            CreateFontW(
                height,
                0,
                0,
                0,
                weight as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                CLEARTYPE_QUALITY as u32,
                (DEFAULT_PITCH | FF_DONTCARE) as u32,
                font_ptr,
            )
        };
        (make_font(-15, FW_SEMIBOLD), make_font(-16, FW_BOLD))
    })
}

// Shared Edit Control Bounding Boxes (20px inner edit centered inside 26px painted rounded pills)
const RECT_EDIT_HR: (i32, i32, i32, i32) = (26, 102, 42, 20);
const RECT_EDIT_MIN: (i32, i32, i32, i32) = (102, 102, 42, 20);
const RECT_EDIT_SEC: (i32, i32, i32, i32) = (188, 102, 42, 20);
const RECT_EDIT_MS: (i32, i32, i32, i32) = (272, 102, 58, 20);
const RECT_EDIT_RATE: (i32, i32, i32, i32) = (26, 102, 72, 20);
const RECT_EDIT_AMOUNT: (i32, i32, i32, i32) = (270, 243, 74, 20);
const RECT_EDIT_LOC_X: (i32, i32, i32, i32) = (293, 195, 42, 20);
const RECT_EDIT_LOC_Y: (i32, i32, i32, i32) = (363, 195, 42, 20);

static HOVERED_BTN: AtomicUsize = AtomicUsize::new(0);

const WM_MOUSELEAVE_U32: u32 = 0x02A3;

unsafe extern "system" fn hover_btn_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let orig = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
    match msg {
        WM_MOUSEMOVE => {
            let cur = hwnd as usize;
            if HOVERED_BTN.load(Ordering::Relaxed) != cur {
                let prev = HOVERED_BTN.swap(cur, Ordering::Relaxed);
                if prev != 0 {
                    InvalidateRect(prev as HWND, null_mut(), 0);
                }
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                TrackMouseEvent(&mut tme);
                InvalidateRect(hwnd, null_mut(), 0);
            }
        }
        WM_MOUSELEAVE_U32 => {
            let cur = hwnd as usize;
            if HOVERED_BTN
                .compare_exchange(cur, 0, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                InvalidateRect(hwnd, null_mut(), 0);
            }
        }
        WM_DESTROY => {
            let cur = hwnd as usize;
            let _ = HOVERED_BTN.compare_exchange(cur, 0, Ordering::Relaxed, Ordering::Relaxed);
        }
        _ => {}
    }
    if orig != 0 {
        let orig_proc: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT =
            std::mem::transmute(orig);
        CallWindowProcW(Some(orig_proc), hwnd, msg, wparam, lparam)
    } else {
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

unsafe fn attach_hover_tracking(hwnd: HWND) {
    let prev = SetWindowLongPtrW(
        hwnd,
        GWLP_WNDPROC,
        hover_btn_proc as *const () as usize as isize,
    );
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, prev);
}

#[inline]
unsafe fn create_child(
    class_ptr: *const u16,
    text: Option<&str>,
    style: u32,
    (x, y, w, h): (i32, i32, i32, i32),
    parent: HWND,
    id: usize,
    hinstance: HINSTANCE,
) -> HWND {
    let hmenu = if id == 0 { null_mut() } else { id as HMENU };
    match text {
        Some(t) => with_wide_null(t, |text_ptr| {
            CreateWindowExW(0, class_ptr, text_ptr, style, x, y, w, h, parent, hmenu, hinstance, null_mut())
        }),
        None => CreateWindowExW(0, class_ptr, null_mut(), style, x, y, w, h, parent, hmenu, hinstance, null_mut()),
    }
}

pub unsafe fn create_controls(
    parent: HWND,
    hinstance: HINSTANCE,
    font_reg: HFONT,
    _font_bold: HFONT,
) -> Controls {
    let btn_class = to_wide_null("BUTTON");
    let edit_class = to_wide_null("EDIT");
    let static_class = to_wide_null("STATIC");
    let combo_class = to_wide_null("COMBOBOX");

    let lbl_style = WS_CHILD | WS_VISIBLE | SS_CENTERIMAGE_STYLE;
    let lbl_click_style = lbl_style | SS_NOTIFY_STYLE;
    let radio_style = WS_CHILD | WS_VISIBLE | BS_AUTORADIOBUTTON_U32 | WS_TABSTOP;
    let combo_style = WS_CHILD | WS_VISIBLE | CBS_DROPDOWNLIST_U32 | WS_VSCROLL | WS_TABSTOP;
    let edit_num_style = WS_CHILD | WS_VISIBLE | ES_CENTER_U32 | ES_NUMBER_U32 | WS_TABSTOP;
    let owner_btn_style = WS_CHILD | WS_VISIBLE | BS_OWNERDRAW_U32 | WS_TABSTOP;

    // 1. Top Action Bar (Owner-drawn modern buttons + Trigger mode combo)
    let btn_start = create_child(btn_class.as_ptr(), Some("START (F6)"), owner_btn_style, (14, 14, 196, 36), parent, ID_BTN_START, hinstance);
    let btn_hotkeys = create_child(btn_class.as_ptr(), Some("Set Hotkey"), owner_btn_style, (218, 14, 108, 36), parent, ID_BTN_HOTKEYS, hinstance);
    let combo_trigger_mode = create_child(combo_class.as_ptr(), None, combo_style, (334, 18, 172, 140), parent, ID_COMBO_TRIGGER_MODE, hinstance);
    update_trigger_mode_options(combo_trigger_mode, 0, 0);

    // 2. Card 1 - Timing Mode (x=14..506, y=60..138)
    let lbl_timing = create_child(static_class.as_ptr(), Some("Timing"), lbl_style, (26, 69, 64, 22), parent, 0, hinstance);
    let radio_rate = create_child(btn_class.as_ptr(), None, radio_style | WS_GROUP, (96, 69, 18, 22), parent, ID_RADIO_RATE, hinstance);
    let lbl_radio_rate = create_child(static_class.as_ptr(), Some("Click Rate"), lbl_click_style, (116, 69, 82, 22), parent, ID_LBL_RADIO_RATE, hinstance);
    let radio_interval = create_child(btn_class.as_ptr(), None, radio_style, (210, 69, 18, 22), parent, ID_RADIO_INTERVAL, hinstance);
    let lbl_radio_interval = create_child(static_class.as_ptr(), Some("Click Interval"), lbl_click_style, (230, 69, 104, 22), parent, ID_LBL_RADIO_INTERVAL, hinstance);

    // Interval Edit controls (20px inner edit centered inside 26px rounded pill)
    let edit_hr = create_child(edit_class.as_ptr(), Some("0"), edit_num_style, RECT_EDIT_HR, parent, ID_EDIT_HR, hinstance);
    let lbl_hr = create_child(static_class.as_ptr(), Some("hr"), lbl_style, (73, 99, 20, 26), parent, 0, hinstance);
    let edit_min = create_child(edit_class.as_ptr(), Some("0"), edit_num_style, RECT_EDIT_MIN, parent, ID_EDIT_MIN, hinstance);
    let lbl_min = create_child(static_class.as_ptr(), Some("min"), lbl_style, (149, 99, 30, 26), parent, 0, hinstance);
    let edit_sec = create_child(edit_class.as_ptr(), Some("0"), edit_num_style, RECT_EDIT_SEC, parent, ID_EDIT_SEC, hinstance);
    let lbl_sec = create_child(static_class.as_ptr(), Some("sec"), lbl_style, (235, 99, 28, 26), parent, 0, hinstance);
    let edit_ms = create_child(edit_class.as_ptr(), Some("100"), edit_num_style, RECT_EDIT_MS, parent, ID_EDIT_MS, hinstance);
    let lbl_ms = create_child(static_class.as_ptr(), Some("ms"), lbl_style, (335, 99, 24, 26), parent, 0, hinstance);

    // Rate Controls (Y: 99)
    let edit_rate = create_child(edit_class.as_ptr(), Some("50"), WS_CHILD | ES_CENTER_U32 | WS_TABSTOP, RECT_EDIT_RATE, parent, ID_EDIT_RATE, hinstance);
    let combo_rate_unit = create_child(combo_class.as_ptr(), None, WS_CHILD | CBS_DROPDOWNLIST_U32 | WS_VSCROLL | WS_TABSTOP, (108, 99, 386, 140), parent, ID_COMBO_RATE_UNIT, hinstance);
    for unit_label in [
        "Clicks / Second (CPS)",
        "Clicks / Minute (CPM)",
        "Clicks / Hour (CPH)",
    ] {
        combobox_add_string(combo_rate_unit, unit_label);
    }
    combobox_set_cursel(combo_rate_unit, 0);

    // 3. Card 2 - Target & Location (x=14..506, y=148..224)
    let lbl_buttons = create_child(static_class.as_ptr(), Some("Target"), lbl_style, (26, 156, 64, 24), parent, 0, hinstance);
    let radio_mouse = create_child(btn_class.as_ptr(), None, radio_style | WS_GROUP, (96, 156, 18, 24), parent, ID_RADIO_MOUSE, hinstance);
    let lbl_radio_mouse = create_child(static_class.as_ptr(), Some("Mouse"), lbl_click_style, (116, 156, 56, 24), parent, ID_LBL_RADIO_MOUSE, hinstance);
    let radio_keyboard = create_child(btn_class.as_ptr(), None, radio_style, (182, 156, 18, 24), parent, ID_RADIO_KEYBOARD, hinstance);
    let lbl_radio_keyboard = create_child(static_class.as_ptr(), Some("Keyboard"), lbl_click_style, (202, 156, 76, 24), parent, ID_LBL_RADIO_KEYBOARD, hinstance);

    let combo_mouse_btn = create_child(combo_class.as_ptr(), None, combo_style, (284, 156, 102, 110), parent, ID_COMBO_MOUSE_BTN, hinstance);
    for btn_label in ["Left", "Right", "Middle"] {
        combobox_add_string(combo_mouse_btn, btn_label);
    }
    combobox_set_cursel(combo_mouse_btn, 0);

    let btn_key_target = create_child(btn_class.as_ptr(), Some("Key: Space"), WS_CHILD | BS_OWNERDRAW_U32 | WS_TABSTOP, (284, 155, 102, 26), parent, ID_BTN_KEY_TARGET, hinstance);
    let combo_click_type = create_child(combo_class.as_ptr(), None, combo_style, (394, 156, 100, 110), parent, ID_COMBO_CLICK_TYPE, hinstance);
    update_click_type_options(combo_click_type, 0, 0);

    // Location Row inside Card 2 (y=192)
    let lbl_location = create_child(static_class.as_ptr(), Some("Location"), lbl_style, (26, 193, 62, 24), parent, 0, hinstance);
    let radio_loc_current = create_child(btn_class.as_ptr(), None, radio_style | WS_GROUP, (94, 193, 18, 24), parent, ID_RADIO_LOC_CURRENT, hinstance);
    let lbl_radio_loc_current = create_child(static_class.as_ptr(), Some("Current"), lbl_click_style, (114, 193, 56, 24), parent, ID_LBL_RADIO_LOC_CURRENT, hinstance);
    let radio_loc_specific = create_child(btn_class.as_ptr(), None, radio_style, (176, 193, 18, 24), parent, ID_RADIO_LOC_SPECIFIC, hinstance);
    let lbl_radio_loc_specific = create_child(static_class.as_ptr(), Some("Specific"), lbl_click_style, (196, 193, 72, 24), parent, ID_LBL_RADIO_LOC_SPECIFIC, hinstance);
    let lbl_loc_x = create_child(static_class.as_ptr(), Some("X:"), lbl_style, (274, 193, 16, 24), parent, 0, hinstance);
    let edit_loc_x = create_child(edit_class.as_ptr(), Some("0"), WS_CHILD | WS_VISIBLE | ES_CENTER_U32 | WS_TABSTOP, RECT_EDIT_LOC_X, parent, ID_EDIT_LOC_X, hinstance);
    let lbl_loc_y = create_child(static_class.as_ptr(), Some("Y:"), lbl_style, (344, 193, 16, 24), parent, 0, hinstance);
    let edit_loc_y = create_child(edit_class.as_ptr(), Some("0"), WS_CHILD | WS_VISIBLE | ES_CENTER_U32 | WS_TABSTOP, RECT_EDIT_LOC_Y, parent, ID_EDIT_LOC_Y, hinstance);
    let btn_pick_pos = create_child(btn_class.as_ptr(), Some("Pick"), owner_btn_style, (416, 192, 78, 26), parent, ID_BTN_PICK_POS, hinstance);

    // 4. Card 3 - Repeat Amount (x=14..506, y=234..272)
    let lbl_amount = create_child(static_class.as_ptr(), Some("Click Amount (0 = forever):"), lbl_style, (26, 241, 236, 24), parent, 0, hinstance);
    let edit_amount = create_child(edit_class.as_ptr(), Some("0"), edit_num_style, RECT_EDIT_AMOUNT, parent, ID_EDIT_AMOUNT, hinstance);

    // Attach smooth hover state tracking to owner-drawn buttons
    for btn in [btn_start, btn_hotkeys, btn_key_target, btn_pick_pos] {
        attach_hover_tracking(btn);
    }

    // Apply native dark mode theme identifiers to controls
    let dark_theme = to_wide_null("DarkMode_Explorer");
    let dark_cfd = to_wide_null("DarkMode_CFD");

    for ctrl in [
        edit_hr, edit_min, edit_sec, edit_ms, edit_rate, edit_amount, edit_loc_x, edit_loc_y,
        radio_interval, radio_rate, radio_mouse, radio_keyboard, radio_loc_current, radio_loc_specific,
    ] {
        SetWindowTheme(ctrl, dark_theme.as_ptr(), null_mut());
    }
    for ctrl in [combo_rate_unit, combo_mouse_btn, combo_click_type, combo_trigger_mode] {
        SetWindowTheme(ctrl, dark_cfd.as_ptr(), null_mut());
    }

    // Set default radio buttons checked (Click Rate, Mouse, Current Location)
    button_set_checked(radio_rate, true);
    button_set_checked(radio_mouse, true);
    button_set_checked(radio_loc_current, true);

    // Apply fonts to controls
    let all_regular = [
        lbl_timing, combo_trigger_mode, radio_interval, lbl_radio_interval, radio_rate, lbl_radio_rate,
        edit_hr, lbl_hr, edit_min, lbl_min, edit_sec, lbl_sec, edit_ms, lbl_ms,
        edit_rate, combo_rate_unit, lbl_buttons, radio_mouse, lbl_radio_mouse,
        radio_keyboard, lbl_radio_keyboard, combo_mouse_btn, combo_click_type,
        lbl_location, radio_loc_current, lbl_radio_loc_current, radio_loc_specific, lbl_radio_loc_specific,
        lbl_loc_x, edit_loc_x, lbl_loc_y, edit_loc_y, lbl_amount, edit_amount,
    ];
    for ctrl in all_regular {
        SendMessageW(ctrl, WM_SETFONT, font_reg as WPARAM, 1);
    }

    // Attach comprehensive Windows tooltips to controls
    let tooltip_hwnd = create_tooltip(parent, hinstance);
    let tooltips = [
        (btn_start, "Start or stop autoclicking (Shortcut: active hotkey)"),
        (btn_hotkeys, "Click to change the global Start/Stop activation hotkey"),
        (combo_trigger_mode, "Start Trigger Mode:\n• Toggle (Hotkey): Press hotkey to start or stop\n• Hold (Hotkey): Autoclicks only while holding hotkey\n• Hold Mouse Click: When enabled, holding mouse click autoclicks"),
        (radio_rate, "Click Rate: Specify clicks per second, minute, or hour (max safe: 100 CPS)"),
        (lbl_radio_rate, "Click Rate: Specify clicks per second, minute, or hour (max safe: 100 CPS)"),
        (edit_rate, "Rate numeric value (Limited to a safe maximum of 100 CPS to prevent app crashes and system slowdowns)"),
        (edit_ms, "Milliseconds delay (Safe limit: minimum 10 ms / 100 CPS to prevent app crashes)"),
        (radio_interval, "Click Interval: Specify exact delay between clicks (hr, min, sec, ms)"),
        (lbl_radio_interval, "Click Interval: Specify exact delay between clicks (hr, min, sec, ms)"),
        (radio_mouse, "Mouse Target: Simulate mouse button clicks"),
        (lbl_radio_mouse, "Mouse Target: Simulate mouse button clicks"),
        (radio_keyboard, "Keyboard Target: Simulate keyboard key presses"),
        (lbl_radio_keyboard, "Keyboard Target: Simulate keyboard key presses"),
        (combo_mouse_btn, "Select mouse button to click (Left, Right, Middle)"),
        (btn_key_target, "Target keyboard key to press or hold (click to change)"),
        (combo_click_type, "Action Type:\n• Mouse: Single click, Double click, or Hold button\n• Keyboard: Press key or Hold key down"),
        (radio_loc_current, "Click wherever the mouse cursor currently is"),
        (lbl_radio_loc_current, "Click wherever the mouse cursor currently is"),
        (radio_loc_specific, "Lock mouse to specific (X, Y) coordinates while clicking"),
        (lbl_radio_loc_specific, "Lock mouse to specific (X, Y) coordinates while clicking"),
        (btn_pick_pos, "Pick screen coordinates by clicking anywhere on screen"),
        (edit_amount, "Repeat count: Total clicks/presses before stopping (0 = forever)"),
    ];
    for (ctrl, tip) in tooltips {
        add_tooltip(tooltip_hwnd, parent, ctrl, tip);
    }

    Controls {
        btn_start,
        btn_hotkeys,
        combo_trigger_mode,
        lbl_timing,
        radio_interval,
        lbl_radio_interval,
        radio_rate,
        lbl_radio_rate,
        edit_hr,
        lbl_hr,
        edit_min,
        lbl_min,
        edit_sec,
        lbl_sec,
        edit_ms,
        lbl_ms,
        edit_rate,
        combo_rate_unit,
        lbl_buttons,
        radio_mouse,
        lbl_radio_mouse,
        radio_keyboard,
        lbl_radio_keyboard,
        combo_mouse_btn,
        combo_click_type,
        btn_key_target,
        lbl_location,
        radio_loc_current,
        lbl_radio_loc_current,
        radio_loc_specific,
        lbl_radio_loc_specific,
        lbl_loc_x,
        edit_loc_x,
        lbl_loc_y,
        edit_loc_y,
        btn_pick_pos,
        lbl_amount,
        edit_amount,
    }
}

/// Paint elevated section cards, subtle dividers, and rounded edit input pills.
/// Uses stock DC_PEN/DC_BRUSH for zero GDI object allocation and 0% idle overhead.
pub unsafe fn draw_ui_chrome(
    hdc: HDC,
    controls: &Controls,
    state: &AppState,
) {
    let pen = GetStockObject(DC_PEN as i32);
    let brush = GetStockObject(DC_BRUSH as i32);
    let old_pen = SelectObject(hdc, pen);
    let old_brush = SelectObject(hdc, brush);

    // 1. Elevated Section Cards
    SetDCPenColor(hdc, COLOR_CARD_BORDER);
    SetDCBrushColor(hdc, COLOR_CARD);
    for (x, y, w, h) in [
        (14, 60, 492, 78),   // Timing card
        (14, 148, 492, 76),  // Target & Location card
        (14, 234, 492, 38),  // Repeat card
    ] {
        RoundRect(hdc, x, y, x + w, y + h, 8, 8);
    }

    // Subtle divider between Target and Location inside Card 2
    SetDCPenColor(hdc, rgb(39, 39, 47));
    MoveToEx(hdc, 26, 186, null_mut());
    LineTo(hdc, 494, 186);

    // 2. Rounded pill frames around visible Edit controls (vertically centering 20px inner edit in 26px pill)
    let edit_list = [
        (controls.edit_hr, RECT_EDIT_HR),
        (controls.edit_min, RECT_EDIT_MIN),
        (controls.edit_sec, RECT_EDIT_SEC),
        (controls.edit_ms, RECT_EDIT_MS),
        (controls.edit_rate, RECT_EDIT_RATE),
        (controls.edit_amount, RECT_EDIT_AMOUNT),
        (controls.edit_loc_x, RECT_EDIT_LOC_X),
        (controls.edit_loc_y, RECT_EDIT_LOC_Y),
    ];

    for (edit, (x, y, w, _h)) in edit_list {
        if IsWindowVisible(edit) != 0 {
            let is_loc_edit = edit == controls.edit_loc_x || edit == controls.edit_loc_y;
            let enabled = !state.is_running
                && (!is_loc_edit || (state.input_mode == 0 && state.location_mode == 1));
            SetDCPenColor(hdc, if enabled { COLOR_BORDER_SUBTLE } else { COLOR_BORDER_DIS });
            SetDCBrushColor(hdc, if enabled { COLOR_SURFACE } else { COLOR_SURFACE_DIS });
            RoundRect(hdc, x - 2, y - 3, x + w + 2, y + 23, 6, 6);
        }
    }

    SelectObject(hdc, old_brush);
    SelectObject(hdc, old_pen);
}

#[inline]
const fn secondary_button_colors(
    is_listening: bool,
    is_disabled: bool,
    is_selected: bool,
    is_hovered: bool,
) -> (u32, u32, u32, bool) {
    if is_listening {
        (rgb(23, 37, 54), rgb(56, 189, 248), rgb(125, 211, 252), true)
    } else if is_disabled {
        (rgb(29, 29, 34), rgb(44, 44, 52), rgb(110, 110, 122), false)
    } else if is_selected {
        (rgb(60, 60, 70), rgb(96, 96, 110), rgb(250, 250, 252), false)
    } else if is_hovered {
        (rgb(50, 50, 58), rgb(84, 84, 96), rgb(244, 244, 247), false)
    } else {
        (rgb(39, 39, 45), rgb(64, 64, 74), rgb(228, 228, 234), false)
    }
}

/// Owner-drawn button renderer for modern dark mode buttons.
/// 0% GPU, 0% CPU idle, zero external framework overhead.
pub unsafe fn draw_custom_button(
    dis: &DRAWITEMSTRUCT,
    state: &AppState,
    font_bold: HFONT,
    font_reg: HFONT,
) {
    let hdc = dis.hDC;
    let mut rc = dis.rcItem;
    let is_selected = (dis.itemState & ODS_SELECTED) != 0;
    let is_disabled = (dis.itemState & ODS_DISABLED) != 0;
    let is_hovered = !is_disabled && HOVERED_BTN.load(Ordering::Relaxed) == dis.hwndItem as usize;
    let ctl_id = dis.CtlID as usize;

    // 1. Flood-fill entire button rect with its surrounding surface color (cards for Pick/Key, canvas for top bar)
    let parent_bg = if matches!(ctl_id, ID_BTN_KEY_TARGET | ID_BTN_PICK_POS) {
        COLOR_CARD
    } else {
        COLOR_BG
    };
    let brush = GetStockObject(DC_BRUSH as i32);
    SetDCBrushColor(hdc, parent_bg);
    FillRect(hdc, &rc, brush as HBRUSH);

    let (bg_color, border_color, text_color, use_bold) = match ctl_id {
        ID_BTN_START => {
            let text_color = rgb(255, 255, 255);
            if is_disabled {
                (rgb(39, 39, 45), rgb(63, 63, 72), rgb(113, 113, 122), true)
            } else if state.is_running {
                let (bg, border) = if is_selected {
                    (rgb(190, 18, 60), rgb(251, 113, 133))
                } else if is_hovered {
                    (rgb(244, 63, 94), rgb(253, 164, 175))
                } else {
                    (rgb(225, 29, 72), rgb(251, 113, 133))
                };
                (bg, border, text_color, true)
            } else {
                let (bg, border) = if is_selected {
                    (rgb(29, 78, 216), rgb(96, 165, 250))
                } else if is_hovered {
                    (rgb(59, 130, 246), rgb(147, 197, 253))
                } else {
                    (rgb(37, 99, 235), rgb(96, 165, 250))
                };
                (bg, border, text_color, true)
            }
        }
        ID_BTN_HOTKEYS => secondary_button_colors(state.rebind_target == RebindTarget::Hotkey, is_disabled, is_selected, is_hovered),
        ID_BTN_KEY_TARGET => secondary_button_colors(state.rebind_target == RebindTarget::TargetKey, is_disabled, is_selected, is_hovered),
        ID_BTN_PICK_POS => secondary_button_colors(state.rebind_target == RebindTarget::PickLocation, is_disabled, is_selected, is_hovered),
        _ => return,
    };

    let pen = GetStockObject(DC_PEN as i32);
    SetDCPenColor(hdc, border_color);
    SetDCBrushColor(hdc, bg_color);

    let old_pen = SelectObject(hdc, pen);
    let old_brush = SelectObject(hdc, brush);

    // Crisp rounded rectangle with 7px corner radius
    RoundRect(hdc, rc.left, rc.top, rc.right, rc.bottom, 7, 7);

    // Accent indicator dot on primary Start/Stop CTA
    if ctl_id == ID_BTN_START {
        let dot_color = if is_disabled {
            rgb(113, 113, 122)
        } else if state.is_running {
            rgb(254, 205, 211)
        } else {
            rgb(191, 219, 254)
        };
        let cy = (rc.top + rc.bottom) / 2;
        SetDCPenColor(hdc, dot_color);
        SetDCBrushColor(hdc, dot_color);
        Ellipse(hdc, rc.left + 14, cy - 4, rc.left + 22, cy + 4);
    }

    SelectObject(hdc, old_brush);
    SelectObject(hdc, old_pen);

    // Draw text centered (with left padding on Start button to balance the indicator dot)
    let mut text_buf = [0u16; 128];
    let len = GetWindowTextW(dis.hwndItem, text_buf.as_mut_ptr(), text_buf.len() as i32);
    if len > 0 {
        SetBkMode(hdc, 1 /* TRANSPARENT */);
        SetBkColor(hdc, bg_color);
        SetTextColor(hdc, text_color);
        let font = if use_bold { font_bold } else { font_reg };
        let old_font = SelectObject(hdc, font);

        if ctl_id == ID_BTN_START {
            rc.left += 10;
        }
        DrawTextW(
            hdc,
            text_buf.as_ptr(),
            len,
            &mut rc,
            (DT_CENTER | DT_VCENTER | DT_SINGLELINE) as u32,
        );

        SelectObject(hdc, old_font);
    }
}

pub fn update_timing_visibility(controls: &Controls, timing_mode: u32) {
    let (show_interval, show_rate) = if timing_mode == 0 {
        (SW_SHOW, SW_HIDE)
    } else {
        (SW_HIDE, SW_SHOW)
    };

    unsafe {
        for ctrl in [
            controls.edit_hr, controls.lbl_hr,
            controls.edit_min, controls.lbl_min,
            controls.edit_sec, controls.lbl_sec,
            controls.edit_ms, controls.lbl_ms,
        ] {
            ShowWindow(ctrl, show_interval);
        }
        ShowWindow(controls.edit_rate, show_rate);
        ShowWindow(controls.combo_rate_unit, show_rate);
    }
}

pub fn update_input_visibility(controls: &Controls, input_mode: u32) {
    let (show_mouse, show_keyboard) = if input_mode == 0 {
        (SW_SHOW, SW_HIDE)
    } else {
        (SW_HIDE, SW_SHOW)
    };

    unsafe {
        ShowWindow(controls.combo_mouse_btn, show_mouse);
        ShowWindow(controls.btn_key_target, show_keyboard);
        ShowWindow(controls.combo_click_type, SW_SHOW);
    }
}

pub fn update_location_state(controls: &Controls, input_mode: u32, location_mode: u32) {
    let is_mouse = input_mode == 0;
    let flag_radios = if is_mouse { 1 } else { 0 };
    let flag_inputs = if is_mouse && location_mode == 1 { 1 } else { 0 };

    unsafe {
        // Only disable interactive radio and edit controls; never disable STATIC label controls
        // to prevent Windows from applying ugly 3D-embossed font distortion.
        EnableWindow(controls.radio_loc_current, flag_radios);
        EnableWindow(controls.radio_loc_specific, flag_radios);
        EnableWindow(controls.edit_loc_x, flag_inputs);
        EnableWindow(controls.edit_loc_y, flag_inputs);
        EnableWindow(controls.btn_pick_pos, flag_radios);

        for ctrl in [
            controls.edit_loc_x, controls.edit_loc_y, controls.btn_pick_pos,
            controls.lbl_location, controls.lbl_radio_loc_current, controls.lbl_radio_loc_specific,
            controls.lbl_loc_x, controls.lbl_loc_y,
        ] {
            InvalidateRect(ctrl, null_mut(), 1);
        }
    }
}

pub fn set_controls_enabled(controls: &Controls, enabled: bool) {
    let flag = if enabled { 1 } else { 0 };

    unsafe {
        for ctrl in [
            controls.btn_hotkeys, controls.combo_trigger_mode,
            controls.radio_interval, controls.radio_rate,
            controls.edit_hr, controls.edit_min, controls.edit_sec, controls.edit_ms,
            controls.edit_rate, controls.combo_rate_unit,
            controls.radio_mouse, controls.radio_keyboard,
            controls.combo_mouse_btn, controls.combo_click_type, controls.btn_key_target,
            controls.edit_amount,
        ] {
            EnableWindow(ctrl, flag);
        }

        if !enabled {
            for ctrl in [
                controls.radio_loc_current, controls.radio_loc_specific,
                controls.edit_loc_x, controls.edit_loc_y, controls.btn_pick_pos,
            ] {
                EnableWindow(ctrl, 0);
            }
        }

        for ctrl in [
            controls.btn_start, controls.btn_hotkeys,
            controls.btn_key_target, controls.btn_pick_pos,
        ] {
            InvalidateRect(ctrl, null_mut(), 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_and_get_control_text() {
        unsafe {
            let class_name = to_wide_null("EDIT");
            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                to_wide_null("Initial").as_ptr(),
                WS_POPUP | ES_CENTER_U32 | ES_NUMBER_U32,
                0, 0, 100, 100,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
            );
            assert!(!hwnd.is_null());
            set_control_text(hwnd, "456");
            assert_eq!(get_control_text(hwnd), "456");
            DestroyWindow(hwnd);
        }
    }

    #[test]
    fn test_click_type_keyboard_press_and_hold_mapping() {
        unsafe {
            let class_name = to_wide_null("COMBOBOX");
            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                null_mut(),
                WS_POPUP | CBS_DROPDOWNLIST_U32,
                0, 0, 100, 100,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
            );
            assert!(!hwnd.is_null());

            // 1. Mouse mode: Single (0), Double (1), Hold (2)
            update_click_type_options(hwnd, 0, 0);
            assert_eq!(get_click_type_from_combo(hwnd, 0), 0);
            combobox_set_cursel(hwnd, 1);
            assert_eq!(get_click_type_from_combo(hwnd, 0), 1);
            combobox_set_cursel(hwnd, 2);
            assert_eq!(get_click_type_from_combo(hwnd, 0), 2);

            // 2. Keyboard mode: Press (0), Hold (1 in combo -> 2 in click_type)
            update_click_type_options(hwnd, 1, 0); // press
            assert_eq!(get_click_type_from_combo(hwnd, 1), 0); // press maps to Single (0)
            combobox_set_cursel(hwnd, 1); // hold
            assert_eq!(get_click_type_from_combo(hwnd, 1), 2); // hold maps to Hold (2)

            // 3. Switching from Mouse Hold (2) to Keyboard sets combo to Hold (index 1)
            update_click_type_options(hwnd, 1, 2);
            assert_eq!(combobox_get_cursel(hwnd), 1);
            assert_eq!(get_click_type_from_combo(hwnd, 1), 2);

            // 4. Switching from Keyboard Hold (2) to Mouse sets combo to Hold (index 2)
            update_click_type_options(hwnd, 0, 2);
            assert_eq!(combobox_get_cursel(hwnd), 2);
            assert_eq!(get_click_type_from_combo(hwnd, 0), 2);

            DestroyWindow(hwnd);
        }
    }

    #[test]
    fn test_trigger_mode_options_mapping() {
        unsafe {
            let class_name = to_wide_null("COMBOBOX");
            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                null_mut(),
                WS_POPUP | CBS_DROPDOWNLIST_U32,
                0, 0, 100, 100,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
            );
            assert!(!hwnd.is_null());

            // 1. Mouse mode: Toggle (0), Hold (1), Hold Mouse Click (2) -> 3 items
            update_trigger_mode_options(hwnd, 0, 2);
            let count_mouse = SendMessageW(hwnd, CB_GETCOUNT, 0, 0);
            assert_eq!(count_mouse, 3);
            assert_eq!(combobox_get_cursel(hwnd), 2);

            // 2. Keyboard mode: Toggle (0), Hold (1) -> 2 items (Hold Mouse Click is removed!)
            update_trigger_mode_options(hwnd, 1, 2);
            let count_kbd = SendMessageW(hwnd, CB_GETCOUNT, 0, 0);
            assert_eq!(count_kbd, 2);
            assert_eq!(combobox_get_cursel(hwnd), 1); // clamped to 1

            // 3. Switching back to Mouse restores 3 items
            update_trigger_mode_options(hwnd, 0, 1);
            let count_back = SendMessageW(hwnd, CB_GETCOUNT, 0, 0);
            assert_eq!(count_back, 3);
            assert_eq!(combobox_get_cursel(hwnd), 1);

            DestroyWindow(hwnd);
        }
    }

    #[test]
    fn test_location_controls_no_overlap() {
        unsafe {
            let class_name = to_wide_null("STATIC");
            let hwnd_parent = CreateWindowExW(
                0,
                class_name.as_ptr(),
                null_mut(),
                WS_POPUP,
                0, 0, 500, 300,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
            );
            assert!(!hwnd_parent.is_null());

            let (font_reg, font_bold) = create_ui_fonts();
            let hinst = windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(null_mut());
            let controls = create_controls(hwnd_parent, hinst, font_reg, font_bold);

            let rel_rect = |ctrl: HWND| -> RECT {
                let mut rc: RECT = std::mem::zeroed();
                GetWindowRect(ctrl, &mut rc);
                let mut tl = POINT { x: rc.left, y: rc.top };
                let mut br = POINT { x: rc.right, y: rc.bottom };
                ScreenToClient(hwnd_parent, &mut tl);
                ScreenToClient(hwnd_parent, &mut br);
                RECT { left: tl.x, top: tl.y, right: br.x, bottom: br.y }
            };

            let pairs = [
                (controls.btn_start, controls.btn_hotkeys),
                (controls.btn_hotkeys, controls.combo_trigger_mode),
                (controls.lbl_timing, controls.radio_rate),
                (controls.lbl_radio_rate, controls.radio_interval),
                (controls.lbl_buttons, controls.radio_mouse),
                (controls.lbl_radio_mouse, controls.radio_keyboard),
                (controls.lbl_radio_keyboard, controls.combo_mouse_btn),
                (controls.combo_mouse_btn, controls.combo_click_type),
                (controls.lbl_location, controls.radio_loc_current),
                (controls.lbl_radio_loc_current, controls.radio_loc_specific),
                (controls.lbl_radio_loc_specific, controls.lbl_loc_x),
                (controls.edit_loc_x, controls.lbl_loc_y),
                (controls.edit_loc_y, controls.btn_pick_pos),
                (controls.lbl_amount, controls.edit_amount),
            ];

            for (left_ctrl, right_ctrl) in pairs {
                let a = rel_rect(left_ctrl);
                let b = rel_rect(right_ctrl);
                assert!(
                    a.right <= b.left,
                    "Control right ({}) overlaps adjacent control left ({})",
                    a.right,
                    b.left
                );
            }

            let specific_rect = rel_rect(controls.lbl_radio_loc_specific);
            assert!(
                specific_rect.right - specific_rect.left >= 70,
                "Specific label width ({}) too narrow to fit full text",
                specific_rect.right - specific_rect.left
            );
            let amount_rect = rel_rect(controls.lbl_amount);
            assert!(
                amount_rect.right - amount_rect.left >= 230,
                "Amount label width ({}) too narrow to fit full text",
                amount_rect.right - amount_rect.left
            );
            assert!(rel_rect(controls.combo_trigger_mode).right <= UI_CLIENT_WIDTH);
            assert!(rel_rect(controls.btn_pick_pos).right <= UI_CLIENT_WIDTH);
            assert!(rel_rect(controls.edit_amount).bottom <= UI_CLIENT_HEIGHT);

            DeleteObject(font_reg);
            DeleteObject(font_bold);
            DestroyWindow(hwnd_parent);
        }
    }
}
