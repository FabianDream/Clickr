use std::ptr::null_mut;
use std::sync::Arc;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::Controls::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_F6, VK_SPACE};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::clicker::WorkerSignal;
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

pub const BST_CHECKED_VAL: usize = 1;
pub const BST_UNCHECKED_VAL: usize = 0;
pub const SS_CENTERIMAGE_STYLE: u32 = 0x0200;
pub const SS_NOTIFY_STYLE: u32 = 0x0100;

// Dark Palette Colors
pub const COLOR_BG: u32 = rgb(24, 24, 27);          // Zinc-900 (#18181B)
pub const COLOR_SURFACE: u32 = rgb(39, 39, 42);     // Zinc-800 (#27272A)
pub const COLOR_SURFACE_DIS: u32 = rgb(28, 28, 31); // Dark disabled surface
pub const COLOR_TEXT_PRIMARY: u32 = rgb(244, 244, 245); // Zinc-100
pub const COLOR_TEXT_MUTED: u32 = rgb(161, 161, 170);   // Zinc-400
pub const COLOR_BORDER_SUBTLE: u32 = rgb(63, 63, 70);   // Zinc-700 (#3F3F46)

const BS_OWNERDRAW_U32: u32 = BS_OWNERDRAW as u32;
const BS_AUTORADIOBUTTON_U32: u32 = BS_AUTORADIOBUTTON as u32;
const ES_CENTER_U32: u32 = ES_CENTER as u32;
const ES_NUMBER_U32: u32 = ES_NUMBER as u32;
const CBS_DROPDOWNLIST_U32: u32 = CBS_DROPDOWNLIST as u32;

pub const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

pub fn to_wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub struct AppState {
    pub is_running: bool,
    pub hotkey: u32,
    pub target_key: u16,
    pub rebind_target: RebindTarget,
    pub worker_signal: Option<Arc<WorkerSignal>>,
    pub timing_mode: u32, // 0 = Interval, 1 = Rate
    pub input_mode: u32,  // 0 = Mouse, 1 = Keyboard
    pub pos_x: i32,
    pub pos_y: i32,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            is_running: false,
            hotkey: VK_F6 as u32,
            target_key: VK_SPACE,
            rebind_target: RebindTarget::None,
            worker_signal: None,
            timing_mode: 1, // Default: Click Rate (requested by user)
            input_mode: 0,  // Default: Mouse (requested by user)
            pos_x: -1,
            pos_y: -1,
        }
    }
}

pub struct ThemeBrushes {
    pub bg_brush: HBRUSH,
    pub edit_brush: HBRUSH,
    pub disabled_edit_brush: HBRUSH,
}

impl ThemeBrushes {
    pub unsafe fn new() -> Self {
        Self {
            bg_brush: CreateSolidBrush(COLOR_BG),
            edit_brush: CreateSolidBrush(COLOR_SURFACE),
            disabled_edit_brush: CreateSolidBrush(COLOR_SURFACE_DIS),
        }
    }

    pub unsafe fn destroy(&self) {
        DeleteObject(self.bg_brush);
        DeleteObject(self.edit_brush);
        DeleteObject(self.disabled_edit_brush);
    }
}

pub struct Controls {
    pub btn_start: HWND,
    pub btn_hotkeys: HWND,

    // Row 1 - Timing
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
    pub btn_key_target: HWND,

    // Row 3 - Amount
    pub lbl_amount: HWND,
    pub edit_amount: HWND,
}

pub fn get_control_text(hwnd: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; (len + 1) as usize];
        let copied = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        if copied > 0 {
            String::from_utf16_lossy(&buf[..copied as usize])
        } else {
            String::new()
        }
    }
}

pub fn set_control_text(hwnd: HWND, text: &str) {
    let wide = to_wide_null(text);
    unsafe {
        SetWindowTextW(hwnd, wide.as_ptr());
        InvalidateRect(hwnd, null_mut(), 1);
    }
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
    let wide = to_wide_null(text);
    unsafe {
        SendMessageW(hwnd, CB_ADDSTRING, 0, wide.as_ptr() as LPARAM);
    }
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

pub unsafe fn create_ui_fonts() -> (HFONT, HFONT) {
    let font_name = to_wide_null("Segoe UI");
    let font_reg = CreateFontW(
        -15,
        0,
        0,
        0,
        FW_SEMIBOLD as i32,
        0,
        0,
        0,
        DEFAULT_CHARSET as u32,
        OUT_DEFAULT_PRECIS as u32,
        CLIP_DEFAULT_PRECIS as u32,
        CLEARTYPE_QUALITY as u32,
        (DEFAULT_PITCH | FF_DONTCARE) as u32,
        font_name.as_ptr(),
    );
    let font_bold = CreateFontW(
        -16,
        0,
        0,
        0,
        FW_BOLD as i32,
        0,
        0,
        0,
        DEFAULT_CHARSET as u32,
        OUT_DEFAULT_PRECIS as u32,
        CLIP_DEFAULT_PRECIS as u32,
        CLEARTYPE_QUALITY as u32,
        (DEFAULT_PITCH | FF_DONTCARE) as u32,
        font_name.as_ptr(),
    );
    (font_reg, font_bold)
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

    // 1. Top Action Bar (Owner-drawn modern buttons)
    let btn_start = CreateWindowExW(
        0,
        btn_class.as_ptr(),
        to_wide_null("START (F6)").as_ptr(),
        WS_CHILD | WS_VISIBLE | BS_OWNERDRAW_U32 | WS_TABSTOP,
        12,
        12,
        310,
        34,
        parent,
        ID_BTN_START as HMENU,
        hinstance,
        null_mut(),
    );

    let btn_hotkeys = CreateWindowExW(
        0,
        btn_class.as_ptr(),
        to_wide_null("Hotkeys").as_ptr(),
        WS_CHILD | WS_VISIBLE | BS_OWNERDRAW_U32 | WS_TABSTOP,
        330,
        12,
        118,
        34,
        parent,
        ID_BTN_HOTKEYS as HMENU,
        hinstance,
        null_mut(),
    );

    // 2. Row 1 - Timing Mode
    let radio_interval = CreateWindowExW(
        0,
        btn_class.as_ptr(),
        null_mut(),
        WS_CHILD | WS_VISIBLE | BS_AUTORADIOBUTTON_U32 | WS_GROUP | WS_TABSTOP,
        12,
        56,
        18,
        22,
        parent,
        ID_RADIO_INTERVAL as HMENU,
        hinstance,
        null_mut(),
    );

    let lbl_radio_interval = CreateWindowExW(
        0,
        static_class.as_ptr(),
        to_wide_null("Click Interval").as_ptr(),
        WS_CHILD | WS_VISIBLE | SS_CENTERIMAGE_STYLE | SS_NOTIFY_STYLE,
        34,
        56,
        110,
        22,
        parent,
        ID_LBL_RADIO_INTERVAL as HMENU,
        hinstance,
        null_mut(),
    );

    let radio_rate = CreateWindowExW(
        0,
        btn_class.as_ptr(),
        null_mut(),
        WS_CHILD | WS_VISIBLE | BS_AUTORADIOBUTTON_U32 | WS_TABSTOP,
        152,
        56,
        18,
        22,
        parent,
        ID_RADIO_RATE as HMENU,
        hinstance,
        null_mut(),
    );

    let lbl_radio_rate = CreateWindowExW(
        0,
        static_class.as_ptr(),
        to_wide_null("Click Rate").as_ptr(),
        WS_CHILD | WS_VISIBLE | SS_CENTERIMAGE_STYLE | SS_NOTIFY_STYLE,
        174,
        56,
        95,
        22,
        parent,
        ID_LBL_RADIO_RATE as HMENU,
        hinstance,
        null_mut(),
    );

    // Flat Edit controls (dwExStyle = 0 and NO WS_BORDER to completely eliminate ugly white 3D sunken borders)
    let edit_hr = CreateWindowExW(
        0,
        edit_class.as_ptr(),
        to_wide_null("0").as_ptr(),
        WS_CHILD | WS_VISIBLE | ES_CENTER_U32 | ES_NUMBER_U32 | WS_TABSTOP,
        12,
        84,
        38,
        24,
        parent,
        ID_EDIT_HR as HMENU,
        hinstance,
        null_mut(),
    );

    let lbl_hr = CreateWindowExW(
        0,
        static_class.as_ptr(),
        to_wide_null("hr").as_ptr(),
        WS_CHILD | WS_VISIBLE | SS_CENTERIMAGE_STYLE,
        53,
        84,
        22,
        24,
        parent,
        null_mut(),
        hinstance,
        null_mut(),
    );

    let edit_min = CreateWindowExW(
        0,
        edit_class.as_ptr(),
        to_wide_null("0").as_ptr(),
        WS_CHILD | WS_VISIBLE | ES_CENTER_U32 | ES_NUMBER_U32 | WS_TABSTOP,
        80,
        84,
        38,
        24,
        parent,
        ID_EDIT_MIN as HMENU,
        hinstance,
        null_mut(),
    );

    let lbl_min = CreateWindowExW(
        0,
        static_class.as_ptr(),
        to_wide_null("min").as_ptr(),
        WS_CHILD | WS_VISIBLE | SS_CENTERIMAGE_STYLE,
        122,
        84,
        32,
        24,
        parent,
        null_mut(),
        hinstance,
        null_mut(),
    );

    let edit_sec = CreateWindowExW(
        0,
        edit_class.as_ptr(),
        to_wide_null("0").as_ptr(),
        WS_CHILD | WS_VISIBLE | ES_CENTER_U32 | ES_NUMBER_U32 | WS_TABSTOP,
        160,
        84,
        38,
        24,
        parent,
        ID_EDIT_SEC as HMENU,
        hinstance,
        null_mut(),
    );

    let lbl_sec = CreateWindowExW(
        0,
        static_class.as_ptr(),
        to_wide_null("sec").as_ptr(),
        WS_CHILD | WS_VISIBLE | SS_CENTERIMAGE_STYLE,
        202,
        84,
        28,
        24,
        parent,
        null_mut(),
        hinstance,
        null_mut(),
    );

    let edit_ms = CreateWindowExW(
        0,
        edit_class.as_ptr(),
        to_wide_null("100").as_ptr(),
        WS_CHILD | WS_VISIBLE | ES_CENTER_U32 | ES_NUMBER_U32 | WS_TABSTOP,
        236,
        84,
        54,
        24,
        parent,
        ID_EDIT_MS as HMENU,
        hinstance,
        null_mut(),
    );

    let lbl_ms = CreateWindowExW(
        0,
        static_class.as_ptr(),
        to_wide_null("ms").as_ptr(),
        WS_CHILD | WS_VISIBLE | SS_CENTERIMAGE_STYLE,
        294,
        84,
        26,
        24,
        parent,
        null_mut(),
        hinstance,
        null_mut(),
    );

    // Rate Controls (Y: 84, initially shown by default)
    let edit_rate = CreateWindowExW(
        0,
        edit_class.as_ptr(),
        to_wide_null("50").as_ptr(),
        WS_CHILD | ES_CENTER_U32 | WS_TABSTOP,
        12,
        84,
        65,
        24,
        parent,
        ID_EDIT_RATE as HMENU,
        hinstance,
        null_mut(),
    );

    let combo_rate_unit = CreateWindowExW(
        0,
        combo_class.as_ptr(),
        null_mut(),
        WS_CHILD | CBS_DROPDOWNLIST_U32 | WS_VSCROLL | WS_TABSTOP,
        85,
        84,
        363,
        140,
        parent,
        ID_COMBO_RATE_UNIT as HMENU,
        hinstance,
        null_mut(),
    );

    combobox_add_string(combo_rate_unit, "Clicks / Second (CPS)");
    combobox_add_string(combo_rate_unit, "Clicks / Minute (CPM)");
    combobox_add_string(combo_rate_unit, "Clicks / Hour (CPH)");
    combobox_set_cursel(combo_rate_unit, 0);

    // 3. Row 2 - Buttons (Input Selection)
    let lbl_buttons = CreateWindowExW(
        0,
        static_class.as_ptr(),
        to_wide_null("Buttons:").as_ptr(),
        WS_CHILD | WS_VISIBLE | SS_CENTERIMAGE_STYLE,
        12,
        122,
        60,
        24,
        parent,
        null_mut(),
        hinstance,
        null_mut(),
    );

    let radio_mouse = CreateWindowExW(
        0,
        btn_class.as_ptr(),
        null_mut(),
        WS_CHILD | WS_VISIBLE | BS_AUTORADIOBUTTON_U32 | WS_GROUP | WS_TABSTOP,
        74,
        122,
        18,
        24,
        parent,
        ID_RADIO_MOUSE as HMENU,
        hinstance,
        null_mut(),
    );

    let lbl_radio_mouse = CreateWindowExW(
        0,
        static_class.as_ptr(),
        to_wide_null("Mouse").as_ptr(),
        WS_CHILD | WS_VISIBLE | SS_CENTERIMAGE_STYLE | SS_NOTIFY_STYLE,
        96,
        122,
        55,
        24,
        parent,
        ID_LBL_RADIO_MOUSE as HMENU,
        hinstance,
        null_mut(),
    );

    let radio_keyboard = CreateWindowExW(
        0,
        btn_class.as_ptr(),
        null_mut(),
        WS_CHILD | WS_VISIBLE | BS_AUTORADIOBUTTON_U32 | WS_TABSTOP,
        156,
        122,
        18,
        24,
        parent,
        ID_RADIO_KEYBOARD as HMENU,
        hinstance,
        null_mut(),
    );

    let lbl_radio_keyboard = CreateWindowExW(
        0,
        static_class.as_ptr(),
        to_wide_null("Keyboard").as_ptr(),
        WS_CHILD | WS_VISIBLE | SS_CENTERIMAGE_STYLE | SS_NOTIFY_STYLE,
        178,
        122,
        76,
        24,
        parent,
        ID_LBL_RADIO_KEYBOARD as HMENU,
        hinstance,
        null_mut(),
    );

    let combo_mouse_btn = CreateWindowExW(
        0,
        combo_class.as_ptr(),
        null_mut(),
        WS_CHILD | WS_VISIBLE | CBS_DROPDOWNLIST_U32 | WS_VSCROLL | WS_TABSTOP,
        260,
        122,
        188,
        110,
        parent,
        ID_COMBO_MOUSE_BTN as HMENU,
        hinstance,
        null_mut(),
    );

    combobox_add_string(combo_mouse_btn, "Left");
    combobox_add_string(combo_mouse_btn, "Right");
    combobox_add_string(combo_mouse_btn, "Middle");
    combobox_set_cursel(combo_mouse_btn, 0);

    let btn_key_target = CreateWindowExW(
        0,
        btn_class.as_ptr(),
        to_wide_null("[ Space ]").as_ptr(),
        WS_CHILD | BS_OWNERDRAW_U32 | WS_TABSTOP,
        260,
        122,
        188,
        26,
        parent,
        ID_BTN_KEY_TARGET as HMENU,
        hinstance,
        null_mut(),
    );

    // 4. Row 3 - Click Amount
    let lbl_amount = CreateWindowExW(
        0,
        static_class.as_ptr(),
        to_wide_null("Click Amount (0 = forever):").as_ptr(),
        WS_CHILD | WS_VISIBLE | SS_CENTERIMAGE_STYLE,
        12,
        160,
        215,
        24,
        parent,
        null_mut(),
        hinstance,
        null_mut(),
    );

    let edit_amount = CreateWindowExW(
        0,
        edit_class.as_ptr(),
        to_wide_null("0").as_ptr(),
        WS_CHILD | WS_VISIBLE | ES_CENTER_U32 | ES_NUMBER_U32 | WS_TABSTOP,
        232,
        160,
        75,
        24,
        parent,
        ID_EDIT_AMOUNT as HMENU,
        hinstance,
        null_mut(),
    );

    // Apply native dark mode theme identifiers to controls
    let dark_theme = to_wide_null("DarkMode_Explorer");
    let dark_cfd = to_wide_null("DarkMode_CFD");

    SetWindowTheme(edit_hr, dark_theme.as_ptr(), null_mut());
    SetWindowTheme(edit_min, dark_theme.as_ptr(), null_mut());
    SetWindowTheme(edit_sec, dark_theme.as_ptr(), null_mut());
    SetWindowTheme(edit_ms, dark_theme.as_ptr(), null_mut());
    SetWindowTheme(edit_rate, dark_theme.as_ptr(), null_mut());
    SetWindowTheme(edit_amount, dark_theme.as_ptr(), null_mut());

    SetWindowTheme(combo_rate_unit, dark_cfd.as_ptr(), null_mut());
    SetWindowTheme(combo_mouse_btn, dark_cfd.as_ptr(), null_mut());

    SetWindowTheme(radio_interval, dark_theme.as_ptr(), null_mut());
    SetWindowTheme(radio_rate, dark_theme.as_ptr(), null_mut());
    SetWindowTheme(radio_mouse, dark_theme.as_ptr(), null_mut());
    SetWindowTheme(radio_keyboard, dark_theme.as_ptr(), null_mut());

    // Set default radio buttons checked (Click Rate & Mouse as requested by user)
    button_set_checked(radio_rate, true);
    button_set_checked(radio_mouse, true);

    // Apply fonts to controls
    let all_regular = [
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
        lbl_amount,
        edit_amount,
    ];

    for ctrl in all_regular {
        SendMessageW(ctrl, WM_SETFONT, font_reg as WPARAM, 1);
    }

    Controls {
        btn_start,
        btn_hotkeys,
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
        btn_key_target,
        lbl_amount,
        edit_amount,
    }
}

/// Draw subtle, elegant Zinc-700 borders around each visible edit control.
/// Eliminates the stark white 3D sunken borders completely.
pub unsafe fn draw_ui_borders(hdc: HDC, parent: HWND, controls: &Controls) {
    let pen = CreatePen(PS_SOLID as i32, 1, COLOR_BORDER_SUBTLE);
    let old_pen = SelectObject(hdc, pen);
    let old_brush = SelectObject(hdc, GetStockObject(NULL_BRUSH as i32));

    let edit_list = [
        controls.edit_hr,
        controls.edit_min,
        controls.edit_sec,
        controls.edit_ms,
        controls.edit_rate,
        controls.edit_amount,
    ];

    for &edit in &edit_list {
        if IsWindowVisible(edit) != 0 {
            let mut rc: RECT = std::mem::zeroed();
            GetWindowRect(edit, &mut rc);
            let mut pt_tl = POINT { x: rc.left, y: rc.top };
            let mut pt_br = POINT { x: rc.right, y: rc.bottom };
            ScreenToClient(parent, &mut pt_tl);
            ScreenToClient(parent, &mut pt_br);

            // 1px border with 4px rounded corners matching the ComboBox
            RoundRect(hdc, pt_tl.x - 1, pt_tl.y - 1, pt_br.x + 1, pt_br.y + 1, 4, 4);
        }
    }

    SelectObject(hdc, old_brush);
    SelectObject(hdc, old_pen);
    DeleteObject(pen);
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

    // 1. Flood-fill entire button rect with parent background color first!
    // This completely eliminates any white corner ticks or white line artifacts!
    let bg_brush = CreateSolidBrush(COLOR_BG);
    FillRect(hdc, &rc, bg_brush);
    DeleteObject(bg_brush);

    let (bg_color, border_color, text_color, use_bold) = match dis.CtlID as usize {
        ID_BTN_START => {
            let use_bold = true;
            let text_color = rgb(255, 255, 255);
            if is_disabled {
                (rgb(39, 39, 42), rgb(63, 63, 70), rgb(113, 113, 122), use_bold)
            } else if state.is_running {
                // Running: Modern Rose/Crimson theme
                let bg = if is_selected {
                    rgb(190, 18, 60)
                } else {
                    rgb(225, 29, 72)
                };
                let border = rgb(251, 113, 133);
                (bg, border, text_color, use_bold)
            } else {
                // Stopped: Modern Electric Blue theme
                let bg = if is_selected {
                    rgb(29, 78, 216)
                } else {
                    rgb(37, 99, 235)
                };
                let border = rgb(59, 130, 246);
                (bg, border, text_color, use_bold)
            }
        }
        ID_BTN_HOTKEYS => {
            let is_listening = state.rebind_target == RebindTarget::Hotkey;
            if is_listening {
                (rgb(30, 41, 59), rgb(56, 189, 248), rgb(56, 189, 248), true)
            } else if is_disabled {
                (rgb(30, 30, 33), rgb(45, 45, 50), rgb(113, 113, 122), false)
            } else if is_selected {
                (rgb(50, 50, 56), rgb(82, 82, 91), rgb(244, 244, 245), false)
            } else {
                (rgb(39, 39, 42), rgb(63, 63, 70), rgb(228, 228, 231), false)
            }
        }
        ID_BTN_KEY_TARGET => {
            let is_listening = state.rebind_target == RebindTarget::TargetKey;
            if is_listening {
                (rgb(30, 41, 59), rgb(56, 189, 248), rgb(56, 189, 248), true)
            } else if is_disabled {
                (rgb(30, 30, 33), rgb(45, 45, 50), rgb(113, 113, 122), false)
            } else if is_selected {
                (rgb(50, 50, 56), rgb(82, 82, 91), rgb(244, 244, 245), false)
            } else {
                (rgb(39, 39, 42), rgb(63, 63, 70), rgb(228, 228, 231), false)
            }
        }
        _ => return,
    };

    let hpen = CreatePen(PS_SOLID as i32, 1, border_color);
    let hbrush = CreateSolidBrush(bg_color);

    let old_pen = SelectObject(hdc, hpen);
    let old_brush = SelectObject(hdc, hbrush);

    // Crisp rounded rectangle with 6px corner radius
    RoundRect(hdc, rc.left, rc.top, rc.right, rc.bottom, 6, 6);

    SelectObject(hdc, old_brush);
    SelectObject(hdc, old_pen);
    DeleteObject(hbrush);
    DeleteObject(hpen);

    // Draw text centered
    let mut text_buf = [0u16; 128];
    let len = GetWindowTextW(dis.hwndItem, text_buf.as_mut_ptr(), text_buf.len() as i32);
    if len > 0 {
        SetBkMode(hdc, 1 /* TRANSPARENT */);
        SetBkColor(hdc, bg_color);
        SetTextColor(hdc, text_color);
        let font = if use_bold { font_bold } else { font_reg };
        let old_font = SelectObject(hdc, font);

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
    let show_interval = if timing_mode == 0 {
        SW_SHOW
    } else {
        SW_HIDE
    };
    let show_rate = if timing_mode == 0 {
        SW_HIDE
    } else {
        SW_SHOW
    };

    unsafe {
        ShowWindow(controls.edit_hr, show_interval);
        ShowWindow(controls.lbl_hr, show_interval);
        ShowWindow(controls.edit_min, show_interval);
        ShowWindow(controls.lbl_min, show_interval);
        ShowWindow(controls.edit_sec, show_interval);
        ShowWindow(controls.lbl_sec, show_interval);
        ShowWindow(controls.edit_ms, show_interval);
        ShowWindow(controls.lbl_ms, show_interval);

        ShowWindow(controls.edit_rate, show_rate);
        ShowWindow(controls.combo_rate_unit, show_rate);
    }
}

pub fn update_input_visibility(controls: &Controls, input_mode: u32) {
    let show_mouse = if input_mode == 0 { SW_SHOW } else { SW_HIDE };
    let show_keyboard = if input_mode == 0 {
        SW_HIDE
    } else {
        SW_SHOW
    };

    unsafe {
        ShowWindow(controls.combo_mouse_btn, show_mouse);
        ShowWindow(controls.btn_key_target, show_keyboard);
    }
}

pub fn set_controls_enabled(controls: &Controls, enabled: bool) {
    let flag = if enabled { 1 } else { 0 };

    unsafe {
        EnableWindow(controls.btn_hotkeys, flag);
        EnableWindow(controls.radio_interval, flag);
        EnableWindow(controls.lbl_radio_interval, flag);
        EnableWindow(controls.radio_rate, flag);
        EnableWindow(controls.lbl_radio_rate, flag);

        EnableWindow(controls.edit_hr, flag);
        EnableWindow(controls.edit_min, flag);
        EnableWindow(controls.edit_sec, flag);
        EnableWindow(controls.edit_ms, flag);

        EnableWindow(controls.edit_rate, flag);
        EnableWindow(controls.combo_rate_unit, flag);

        EnableWindow(controls.radio_mouse, flag);
        EnableWindow(controls.lbl_radio_mouse, flag);
        EnableWindow(controls.radio_keyboard, flag);
        EnableWindow(controls.lbl_radio_keyboard, flag);
        EnableWindow(controls.combo_mouse_btn, flag);
        EnableWindow(controls.btn_key_target, flag);

        EnableWindow(controls.edit_amount, flag);

        // Invalidate custom-drawn buttons to repaint their disabled/enabled visuals
        InvalidateRect(controls.btn_start, null_mut(), 1);
        InvalidateRect(controls.btn_hotkeys, null_mut(), 1);
        InvalidateRect(controls.btn_key_target, null_mut(), 1);
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
}
