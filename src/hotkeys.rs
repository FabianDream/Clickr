use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::clicker::WorkerSignal;

pub const WM_APP_KEY_CAPTURED: u32 = 0x8002;
pub const WM_APP_LOCATION_CAPTURED: u32 = 0x8003;
pub const HOTKEY_ID_START_STOP: i32 = 1;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RebindTarget {
    None = 0,
    Hotkey = 1,
    TargetKey = 2,
    PickLocation = 3,
}

static REBIND_TARGET: AtomicU32 = AtomicU32::new(0);
static HOOK_HANDLE: AtomicIsize = AtomicIsize::new(0);
static MOUSE_HOOK_HANDLE: AtomicIsize = AtomicIsize::new(0);
static TARGET_HWND: AtomicIsize = AtomicIsize::new(0);

static MOUSE_MONITOR_ACTIVE: AtomicBool = AtomicBool::new(false);
static MOUSE_MONITOR_BTN: AtomicU32 = AtomicU32::new(0);
static MOUSE_MONITOR_HOOK: AtomicIsize = AtomicIsize::new(0);
static MOUSE_MONITOR_SIGNAL: Mutex<Option<Arc<WorkerSignal>>> = Mutex::new(None);

/// Check if a virtual key code is a modifier key pressed by itself.
pub fn is_lone_modifier(vk: u32) -> bool {
    matches!(
        vk as u16,
        VK_SHIFT
            | VK_LSHIFT
            | VK_RSHIFT
            | VK_CONTROL
            | VK_LCONTROL
            | VK_RCONTROL
            | VK_MENU
            | VK_LMENU
            | VK_RMENU
            | VK_LWIN
            | VK_RWIN
    )
}

/// Convert a virtual key code into a human-readable display string.
pub fn vk_to_string(vk: u32) -> String {
    let vk16 = vk as u16;
    match vk16 {
        VK_F1..=VK_F24 => format!("F{}", vk16 - VK_F1 + 1),
        0x30..=0x39 => format!("{}", (vk16 as u8) as char),
        0x41..=0x5A => format!("{}", (vk16 as u8) as char),
        VK_SPACE => "Space".to_string(),
        VK_RETURN => "Enter".to_string(),
        VK_TAB => "Tab".to_string(),
        VK_ESCAPE => "Esc".to_string(),
        VK_BACK => "Backspace".to_string(),
        VK_DELETE => "Delete".to_string(),
        VK_INSERT => "Insert".to_string(),
        VK_HOME => "Home".to_string(),
        VK_END => "End".to_string(),
        VK_PRIOR => "PageUp".to_string(),
        VK_NEXT => "PageDown".to_string(),
        VK_UP => "Up".to_string(),
        VK_DOWN => "Down".to_string(),
        VK_LEFT => "Left".to_string(),
        VK_RIGHT => "Right".to_string(),
        VK_CAPITAL => "CapsLock".to_string(),
        VK_NUMPAD0..=VK_NUMPAD9 => format!("Num {}", vk16 - VK_NUMPAD0),
        VK_MULTIPLY => "Num *".to_string(),
        VK_ADD => "Num +".to_string(),
        VK_SUBTRACT => "Num -".to_string(),
        VK_DECIMAL => "Num .".to_string(),
        VK_DIVIDE => "Num /".to_string(),
        VK_OEM_1 => ";".to_string(),
        VK_OEM_PLUS => "+".to_string(),
        VK_OEM_COMMA => ",".to_string(),
        VK_OEM_MINUS => "-".to_string(),
        VK_OEM_PERIOD => ".".to_string(),
        VK_OEM_2 => "/".to_string(),
        VK_OEM_3 => "`".to_string(),
        VK_OEM_4 => "[".to_string(),
        VK_OEM_5 => "\\".to_string(),
        VK_OEM_6 => "]".to_string(),
        VK_OEM_7 => "'".to_string(),
        _ => {
            unsafe {
                let scan = MapVirtualKeyW(vk, 0); // MAPVK_VK_TO_VSC = 0
                if scan > 0 {
                    let mut buf = [0u16; 64];
                    let len = GetKeyNameTextW(
                        (scan << 16) as i32,
                        buf.as_mut_ptr(),
                        buf.len() as i32,
                    );
                    if len > 0 {
                        return String::from_utf16_lossy(&buf[..len as usize]);
                    }
                }
            }
            format!("Key 0x{:02X}", vk)
        }
    }
}

/// Low-level keyboard hook callback.
/// Captures the next key pressed when rebinding and notifies the UI thread.
unsafe extern "system" fn low_level_keyboard_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code >= 0 && (wparam as u32 == WM_KEYDOWN || wparam as u32 == WM_SYSKEYDOWN) {
        let kbd = &*(lparam as *const KBDLLHOOKSTRUCT);
        let target = REBIND_TARGET.load(Ordering::SeqCst);

        if target == RebindTarget::PickLocation as u32 {
            let hwnd = TARGET_HWND.load(Ordering::SeqCst) as HWND;
            if hwnd != null_mut() {
                if kbd.vkCode == VK_ESCAPE as u32 {
                    // Cancel pick location
                    PostMessageW(
                        hwnd,
                        WM_APP_KEY_CAPTURED,
                        target as usize,
                        VK_ESCAPE as isize,
                    );
                    return 1;
                } else if kbd.vkCode == VK_SPACE as u32 || kbd.vkCode == VK_RETURN as u32 {
                    // Capture current cursor position on Space or Enter
                    let mut pt: POINT = std::mem::zeroed();
                    GetCursorPos(&mut pt);
                    PostMessageW(
                        hwnd,
                        WM_APP_LOCATION_CAPTURED,
                        pt.x as isize as usize,
                        pt.y as isize,
                    );
                    return 1;
                }
            }
        } else if target != 0 {
            // Ignore lone modifier keys like pressing Shift by itself
            if !is_lone_modifier(kbd.vkCode) {
                let hwnd = TARGET_HWND.load(Ordering::SeqCst) as HWND;
                if hwnd != null_mut() {
                    PostMessageW(
                        hwnd,
                        WM_APP_KEY_CAPTURED,
                        target as usize,
                        kbd.vkCode as isize,
                    );
                    return 1; // Suppress key event from propagating
                }
            }
        }
    }

    CallNextHookEx(null_mut(), code, wparam, lparam)
}

/// Low-level mouse hook callback.
/// Captures mouse clicks across windows when in PickLocation mode.
unsafe extern "system" fn low_level_mouse_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code >= 0 {
        let msg = wparam as u32;
        if msg == WM_LBUTTONDOWN || msg == WM_RBUTTONDOWN || msg == WM_MBUTTONDOWN {
            let target = REBIND_TARGET.load(Ordering::SeqCst);
            if target == RebindTarget::PickLocation as u32 {
                let mouse = &*(lparam as *const MSLLHOOKSTRUCT);
                let hwnd = TARGET_HWND.load(Ordering::SeqCst) as HWND;
                if hwnd != null_mut() {
                    let wnd = WindowFromPoint(mouse.pt);
                    let root = GetAncestor(wnd, GA_ROOT);
                    if wnd == hwnd || root == hwnd {
                        // Clicking inside Clickr cancels the pick
                        PostMessageW(
                            hwnd,
                            WM_APP_KEY_CAPTURED,
                            target as usize,
                            VK_ESCAPE as isize,
                        );
                    } else {
                        // Captured target position outside Clickr
                        PostMessageW(
                            hwnd,
                            WM_APP_LOCATION_CAPTURED,
                            mouse.pt.x as isize as usize,
                            mouse.pt.y as isize,
                        );
                    }
                    return 1; // Suppress click
                }
            }
        }
    }

    CallNextHookEx(null_mut(), code, wparam, lparam)
}

/// Begin capturing a key press for either Hotkey or TargetKey.
pub fn start_rebind(hwnd: HWND, target: RebindTarget) {
    stop_rebind(); // Clean up any existing hook

    REBIND_TARGET.store(target as u32, Ordering::SeqCst);
    TARGET_HWND.store(hwnd as isize, Ordering::SeqCst);

    unsafe {
        let hook = SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(low_level_keyboard_proc),
            null_mut(),
            0,
        );
        HOOK_HANDLE.store(hook as isize, Ordering::SeqCst);
    }
}

/// Begin capturing mouse click or keypress for picking desktop location.
pub fn start_pick_location(hwnd: HWND) {
    stop_rebind();

    REBIND_TARGET.store(RebindTarget::PickLocation as u32, Ordering::SeqCst);
    TARGET_HWND.store(hwnd as isize, Ordering::SeqCst);

    unsafe {
        let kbd_hook = SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(low_level_keyboard_proc),
            null_mut(),
            0,
        );
        HOOK_HANDLE.store(kbd_hook as isize, Ordering::SeqCst);

        let mouse_hook = SetWindowsHookExW(
            WH_MOUSE_LL,
            Some(low_level_mouse_proc),
            null_mut(),
            0,
        );
        MOUSE_HOOK_HANDLE.store(mouse_hook as isize, Ordering::SeqCst);
    }
}

/// Stop capturing key presses / mouse clicks and unhook any active low-level hooks.
pub fn stop_rebind() {
    REBIND_TARGET.store(0, Ordering::SeqCst);
    let hook = HOOK_HANDLE.swap(0, Ordering::SeqCst) as HHOOK;
    if hook != null_mut() {
        unsafe {
            UnhookWindowsHookEx(hook);
        }
    }
    let mouse_hook = MOUSE_HOOK_HANDLE.swap(0, Ordering::SeqCst) as HHOOK;
    if mouse_hook != null_mut() {
        unsafe {
            UnhookWindowsHookEx(mouse_hook);
        }
    }
}

/// Register the global Start/Stop hotkey.
pub fn register_start_stop_hotkey(hwnd: HWND, vk: u32) -> bool {
    unsafe {
        RegisterHotKey(hwnd, HOTKEY_ID_START_STOP, 0, vk) != 0
    }
}

/// Unregister the global Start/Stop hotkey.
pub fn unregister_start_stop_hotkey(hwnd: HWND) {
    unsafe {
        UnregisterHotKey(hwnd, HOTKEY_ID_START_STOP);
    }
}

/// Low-level mouse hook callback for HoldMouse trigger mode.
/// Detects physical hardware mouse button down and up events (filtering out synthetic clicks from SendInput).
unsafe extern "system" fn mouse_monitor_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code >= 0 && MOUSE_MONITOR_ACTIVE.load(Ordering::Relaxed) {
        let mouse = &*(lparam as *const MSLLHOOKSTRUCT);
        // LLMHF_INJECTED is bit 0 of flags. If 0, event originated from real physical mouse hardware!
        if (mouse.flags & 1) == 0 {
            let msg = wparam as u32;
            let btn = MOUSE_MONITOR_BTN.load(Ordering::Relaxed);
            let (down_msg, up_msg) = match btn {
                1 => (WM_RBUTTONDOWN, WM_RBUTTONUP),
                2 => (WM_MBUTTONDOWN, WM_MBUTTONUP),
                _ => (WM_LBUTTONDOWN, WM_LBUTTONUP),
            };

            if msg == down_msg {
                crate::clicker::set_physical_mouse_down(true);
                if let Ok(sig_guard) = MOUSE_MONITOR_SIGNAL.lock() {
                    if let Some(ref signal) = *sig_guard {
                        signal.condvar.notify_all();
                    }
                }
            } else if msg == up_msg {
                crate::clicker::set_physical_mouse_down(false);
                if let Ok(sig_guard) = MOUSE_MONITOR_SIGNAL.lock() {
                    if let Some(ref signal) = *sig_guard {
                        signal.condvar.notify_all();
                    }
                }
            }
        }
    }

    CallNextHookEx(null_mut(), code, wparam, lparam)
}

/// Begin monitoring physical mouse button down/up for HoldMouse trigger mode.
pub fn start_mouse_hold_monitor(btn: u32, signal: Arc<WorkerSignal>) {
    stop_mouse_hold_monitor();
    MOUSE_MONITOR_BTN.store(btn, Ordering::SeqCst);
    if let Ok(mut sig) = MOUSE_MONITOR_SIGNAL.lock() {
        *sig = Some(signal);
    }
    MOUSE_MONITOR_ACTIVE.store(true, Ordering::SeqCst);

    // Check initial physical state of the button
    let vkey = match btn {
        1 => VK_RBUTTON,
        2 => VK_MBUTTON,
        _ => VK_LBUTTON,
    };
    let is_down = unsafe { (GetAsyncKeyState(vkey as i32) as u16 & 0x8000) != 0 };
    crate::clicker::set_physical_mouse_down(is_down);

    unsafe {
        let hook = SetWindowsHookExW(
            WH_MOUSE_LL,
            Some(mouse_monitor_proc),
            null_mut(),
            0,
        );
        MOUSE_MONITOR_HOOK.store(hook as isize, Ordering::SeqCst);
    }
}

/// Stop monitoring physical mouse button and remove low-level mouse hook.
pub fn stop_mouse_hold_monitor() {
    MOUSE_MONITOR_ACTIVE.store(false, Ordering::SeqCst);
    crate::clicker::set_physical_mouse_down(false);
    if let Ok(mut sig) = MOUSE_MONITOR_SIGNAL.lock() {
        if let Some(signal) = sig.take() {
            signal.condvar.notify_all();
        }
    }
    let hook = MOUSE_MONITOR_HOOK.swap(0, Ordering::SeqCst) as HHOOK;
    if hook != null_mut() {
        unsafe {
            UnhookWindowsHookEx(hook);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vk_to_string_function_keys() {
        assert_eq!(vk_to_string(VK_F1 as u32), "F1");
        assert_eq!(vk_to_string(VK_F6 as u32), "F6");
        assert_eq!(vk_to_string(VK_F12 as u32), "F12");
    }

    #[test]
    fn test_vk_to_string_standard_keys() {
        assert_eq!(vk_to_string(VK_SPACE as u32), "Space");
        assert_eq!(vk_to_string(VK_RETURN as u32), "Enter");
        assert_eq!(vk_to_string(VK_TAB as u32), "Tab");
        assert_eq!(vk_to_string(VK_ESCAPE as u32), "Esc");
    }

    #[test]
    fn test_vk_to_string_letters_and_digits() {
        assert_eq!(vk_to_string(b'A' as u32), "A");
        assert_eq!(vk_to_string(b'Z' as u32), "Z");
        assert_eq!(vk_to_string(b'0' as u32), "0");
        assert_eq!(vk_to_string(b'9' as u32), "9");
    }

    #[test]
    fn test_is_lone_modifier() {
        assert!(is_lone_modifier(VK_SHIFT as u32));
        assert!(is_lone_modifier(VK_CONTROL as u32));
        assert!(is_lone_modifier(VK_MENU as u32));
        assert!(is_lone_modifier(VK_LWIN as u32));
        assert!(!is_lone_modifier(VK_F6 as u32));
        assert!(!is_lone_modifier(VK_SPACE as u32));
        assert!(!is_lone_modifier(b'A' as u32));
    }

    #[test]
    fn test_rebind_targets() {
        assert_eq!(RebindTarget::None as u32, 0);
        assert_eq!(RebindTarget::Hotkey as u32, 1);
        assert_eq!(RebindTarget::TargetKey as u32, 2);
        assert_eq!(RebindTarget::PickLocation as u32, 3);
    }

    #[test]
    fn test_mouse_hold_monitor_lifecycle() {
        let signal = Arc::new(WorkerSignal::new());
        start_mouse_hold_monitor(0, signal.clone());
        assert!(MOUSE_MONITOR_ACTIVE.load(Ordering::SeqCst));
        assert_eq!(MOUSE_MONITOR_BTN.load(Ordering::SeqCst), 0);

        stop_mouse_hold_monitor();
        assert!(!MOUSE_MONITOR_ACTIVE.load(Ordering::SeqCst));
        assert!(!crate::clicker::is_physical_mouse_down());
    }
}
