use std::ptr::null_mut;
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub const WM_APP_KEY_CAPTURED: u32 = 0x8002;
pub const HOTKEY_ID_START_STOP: i32 = 1;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RebindTarget {
    None = 0,
    Hotkey = 1,
    TargetKey = 2,
}

static REBIND_TARGET: AtomicU32 = AtomicU32::new(0);
static HOOK_HANDLE: AtomicIsize = AtomicIsize::new(0);
static TARGET_HWND: AtomicIsize = AtomicIsize::new(0);

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

        if target != 0 {
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

/// Stop capturing key presses and unhook the low-level keyboard hook.
pub fn stop_rebind() {
    REBIND_TARGET.store(0, Ordering::SeqCst);
    let hook = HOOK_HANDLE.swap(0, Ordering::SeqCst) as HHOOK;
    if hook != null_mut() {
        unsafe {
            UnhookWindowsHookEx(hook);
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
}
