use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{FreeLibrary, HMODULE, HWND};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::ui::with_wide_null;

pub const GITHUB_REPO: &str = "FabianDream/Clickr";
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

const WINHTTP_ACCESS_TYPE_DEFAULT_PROXY: u32 = 0;
const WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY: u32 = 4;
const WINHTTP_FLAG_SECURE: u32 = 0x0080_0000;
const WINHTTP_OPTION_REDIRECT_POLICY: u32 = 88;
const WINHTTP_OPTION_REDIRECT_POLICY_ALWAYS: u32 = 2;
const WINHTTP_QUERY_CONTENT_LENGTH: u32 = 5;
const WINHTTP_QUERY_STATUS_CODE: u32 = 19;
const WINHTTP_QUERY_FLAG_NUMBER: u32 = 0x2000_0000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
    pub tag: String,
    pub version: String,
    pub download_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    Idle,
    Checking,
    Available(Box<UpdateInfo>),
    Downloading,
    UpToDate,
    Failed(String),
}

struct DynamicLib(HMODULE);

impl DynamicLib {
    unsafe fn load(dll_name: &str) -> Option<Self> {
        let h = with_wide_null(dll_name, |ptr| LoadLibraryW(ptr));
        if h.is_null() { None } else { Some(Self(h)) }
    }

    unsafe fn proc_addr(&self, name: &[u8]) -> Option<*const c_void> {
        GetProcAddress(self.0, name.as_ptr()).map(|f| f as *const c_void)
    }
}

impl Drop for DynamicLib {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                FreeLibrary(self.0);
            }
        }
    }
}

type FnWinHttpCloseHandle = unsafe extern "system" fn(*mut c_void) -> i32;
type FnWinHttpOpen = unsafe extern "system" fn(*const u16, u32, *const u16, *const u16, u32) -> *mut c_void;
type FnWinHttpConnect = unsafe extern "system" fn(*mut c_void, *const u16, u16, u32) -> *mut c_void;
type FnWinHttpOpenRequest = unsafe extern "system" fn(*mut c_void, *const u16, *const u16, *const u16, *const u16, *const *const u16, u32) -> *mut c_void;
type FnWinHttpSetOption = unsafe extern "system" fn(*const c_void, u32, *const c_void, u32) -> i32;
type FnWinHttpSendRequest = unsafe extern "system" fn(*mut c_void, *const u16, u32, *const c_void, u32, u32, usize) -> i32;
type FnWinHttpReceiveResponse = unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32;
type FnWinHttpQueryHeaders = unsafe extern "system" fn(*mut c_void, u32, *const u16, *mut c_void, *mut u32, *mut u32) -> i32;
type FnWinHttpReadData = unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut u32) -> i32;

struct WinHttpHandle {
    handle: *mut c_void,
    close_fn: FnWinHttpCloseHandle,
}

impl Drop for WinHttpHandle {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                (self.close_fn)(self.handle);
            }
        }
    }
}

fn parse_https_url(url: &str) -> Result<(&str, u16, &str), String> {
    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| "Only HTTPS URLs are supported".to_string())?;
    let (authority, path) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => {
            let port = p.parse::<u16>().map_err(|_| "Invalid port in URL".to_string())?;
            (h, port)
        }
        None => (authority, 443),
    };
    if host.is_empty() {
        return Err("Empty host in URL".to_string());
    }
    Ok((host, port, path))
}

/// Download contents of a URL over HTTPS using dynamically loaded native WinHTTP,
/// then unload `winhttp.dll` so network/TLS pages do not stay resident in RAM.
pub fn http_get(url: &str) -> Result<Vec<u8>, String> {
    let (host, port, path) = parse_https_url(url)?;

    unsafe {
        let lib = DynamicLib::load("winhttp.dll")
            .ok_or_else(|| "Failed to load Windows HTTP subsystem (winhttp.dll)".to_string())?;

        macro_rules! load_fn {
            ($name:literal, $ty:ty) => {{
                let ptr = lib
                    .proc_addr($name)
                    .ok_or_else(|| "Missing WinHTTP entry point".to_string())?;
                std::mem::transmute::<*const c_void, $ty>(ptr)
            }};
        }

        let p_open = load_fn!(b"WinHttpOpen\0", FnWinHttpOpen);
        let p_connect = load_fn!(b"WinHttpConnect\0", FnWinHttpConnect);
        let p_open_request = load_fn!(b"WinHttpOpenRequest\0", FnWinHttpOpenRequest);
        let p_set_option = load_fn!(b"WinHttpSetOption\0", FnWinHttpSetOption);
        let p_send_request = load_fn!(b"WinHttpSendRequest\0", FnWinHttpSendRequest);
        let p_receive_response = load_fn!(b"WinHttpReceiveResponse\0", FnWinHttpReceiveResponse);
        let p_query_headers = load_fn!(b"WinHttpQueryHeaders\0", FnWinHttpQueryHeaders);
        let p_read_data = load_fn!(b"WinHttpReadData\0", FnWinHttpReadData);
        let p_close = load_fn!(b"WinHttpCloseHandle\0", FnWinHttpCloseHandle);

        let h_session = with_wide_null("Clickr-AutoUpdater", |ua| {
            let h = p_open(ua, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, null(), null(), 0);
            if h.is_null() {
                p_open(ua, WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, null(), null(), 0)
            } else {
                h
            }
        });
        if h_session.is_null() {
            return Err("Failed to initialize Windows HTTP session".to_string());
        }
        let _session_guard = WinHttpHandle { handle: h_session, close_fn: p_close };

        let h_connect = with_wide_null(host, |host_w| p_connect(h_session, host_w, port, 0));
        if h_connect.is_null() {
            return Err("Failed to establish connection to GitHub".to_string());
        }
        let _connect_guard = WinHttpHandle { handle: h_connect, close_fn: p_close };

        let h_request = with_wide_null("GET", |verb_w| {
            with_wide_null(path, |path_w| {
                p_open_request(
                    h_connect,
                    verb_w,
                    path_w,
                    null(),
                    null(),
                    null(),
                    WINHTTP_FLAG_SECURE,
                )
            })
        });
        if h_request.is_null() {
            return Err("Failed to create HTTP request".to_string());
        }
        let _request_guard = WinHttpHandle { handle: h_request, close_fn: p_close };

        // Allow cross-domain HTTPS redirects (github.com -> objects.githubusercontent.com)
        let redirect_policy = WINHTTP_OPTION_REDIRECT_POLICY_ALWAYS;
        p_set_option(
            h_request,
            WINHTTP_OPTION_REDIRECT_POLICY,
            &redirect_policy as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        );

        let sent = with_wide_null("Accept: */*\r\n", |hdr_w| {
            p_send_request(h_request, hdr_w, u32::MAX, null(), 0, 0, 0)
        });
        if sent == 0 || p_receive_response(h_request, null_mut()) == 0 {
            return Err("Failed to communicate with GitHub".to_string());
        }

        let mut status_code: u32 = 0;
        let mut status_len = std::mem::size_of::<u32>() as u32;
        let mut index = 0u32;
        let query_res = p_query_headers(
            h_request,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            &mut status_code as *mut u32 as *mut c_void,
            &mut status_len,
            &mut index,
        );
        if query_res != 0 && status_code != 200 {
            return Err(format!("GitHub returned HTTP status {}", status_code));
        }

        // Pre-allocate response buffer when Content-Length is available (avoids repeated Vec growth)
        let mut content_len: u32 = 0;
        let mut content_len_size = std::mem::size_of::<u32>() as u32;
        let mut len_index = 0u32;
        let has_len = p_query_headers(
            h_request,
            WINHTTP_QUERY_CONTENT_LENGTH | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            &mut content_len as *mut u32 as *mut c_void,
            &mut content_len_size,
            &mut len_index,
        ) != 0;
        let initial_cap = if has_len {
            (content_len as usize).min(64 * 1024 * 1024)
        } else {
            0
        };

        let mut response = Vec::with_capacity(initial_cap);
        let mut buffer = [0u8; 16384];
        let mut bytes_read: u32 = 0;

        loop {
            let ok = p_read_data(
                h_request,
                buffer.as_mut_ptr() as *mut c_void,
                buffer.len() as u32,
                &mut bytes_read,
            );
            if ok == 0 {
                return Err("Failed while reading response from GitHub".to_string());
            }
            if bytes_read == 0 {
                break;
            }
            response.extend_from_slice(&buffer[..bytes_read as usize]);
        }

        Ok(response)
    }
}

/// Extract string value for a given key in flat/nested JSON without pulling in heavy parser crates.
pub fn extract_json_string(json: &str, key: &str) -> Option<String> {
    let bytes = json.as_bytes();
    let key_bytes = key.as_bytes();
    let mut search_idx = 0;

    while let Some(rel_quote) = json[search_idx..].find('"') {
        let quote_start = search_idx + rel_quote;
        let key_start = quote_start + 1;
        let key_end = key_start + key_bytes.len();
        if key_end < bytes.len()
            && &bytes[key_start..key_end] == key_bytes
            && bytes[key_end] == b'"'
        {
            let after_key = key_end + 1;
            let trimmed_key_after = json[after_key..].trim_start();
            if let Some(after_colon) = trimmed_key_after.strip_prefix(':') {
                let trimmed = after_colon.trim_start();
                if let Some(str_content) = trimmed.strip_prefix('"') {
                    let mut result = String::new();
                    let mut escaped = false;
                    for c in str_content.chars() {
                        if escaped {
                            match c {
                                'n' => result.push('\n'),
                                'r' => result.push('\r'),
                                't' => result.push('\t'),
                                '\\' => result.push('\\'),
                                '"' => result.push('"'),
                                '/' => result.push('/'),
                                _ => {
                                    result.push('\\');
                                    result.push(c);
                                }
                            }
                            escaped = false;
                        } else if c == '\\' {
                            escaped = true;
                        } else if c == '"' {
                            return Some(result);
                        } else {
                            result.push(c);
                        }
                    }
                }
            }
        }
        search_idx = key_start;
    }
    None
}

fn find_ascii_case_insensitive(haystack: &[u8], needle: &[u8], start: usize) -> Option<usize> {
    if needle.is_empty() || start + needle.len() > haystack.len() {
        return None;
    }
    haystack[start..]
        .windows(needle.len())
        .position(|w| w.eq_ignore_ascii_case(needle))
        .map(|rel| start + rel)
}

#[inline]
fn floor_char_boundary(s: &str, mut idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

#[inline]
fn ceil_char_boundary(s: &str, mut idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    while idx < s.len() && !s.is_char_boundary(idx) {
        idx += 1;
    }
    idx
}

/// Find the browser download URL for an asset matching target_name.
pub fn extract_asset_url(json: &str, target_name: &str) -> Option<String> {
    let hay = json.as_bytes();
    let needle = target_name.as_bytes();
    let mut search_idx = 0;

    while let Some(abs_pos) = find_ascii_case_insensitive(hay, needle, search_idx) {
        let window_start = floor_char_boundary(json, abs_pos.saturating_sub(1500));
        let window_end = ceil_char_boundary(json, std::cmp::min(json.len(), abs_pos + 1500));
        let window = &json[window_start..window_end];
        if let Some(url) = extract_json_string(window, "browser_download_url") {
            if url.starts_with("https://") {
                return Some(url);
            }
        }
        search_idx = abs_pos + needle.len();
    }
    None
}

/// Parse release metadata from GitHub API response.
pub fn parse_release_json(json: &str) -> Option<UpdateInfo> {
    let tag = extract_json_string(json, "tag_name")?;
    let version = tag.trim_start_matches(|c| c == 'v' || c == 'V').to_string();
    let download_url = extract_asset_url(json, "Clickr.exe")
        .or_else(|| extract_asset_url(json, ".exe"))?;

    Some(UpdateInfo {
        tag,
        version,
        download_url,
    })
}

/// Parse version string into (major, minor, patch) components.
pub fn parse_version(tag: &str) -> Option<(u64, u64, u64)> {
    let tag = tag.trim().trim_start_matches(|c| c == 'v' || c == 'V');
    let mut parts = tag.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

/// Compare two versions semantically. Returns true if remote is strictly newer than current.
pub fn is_newer_version(remote_tag: &str, current_tag: &str) -> bool {
    match (parse_version(remote_tag), parse_version(current_tag)) {
        (Some(remote), Some(current)) => remote > current,
        _ => false,
    }
}

/// Check GitHub releases for the latest version.
pub fn check_for_updates() -> Result<Option<UpdateInfo>, String> {
    let url = format!("https://api.github.com/repos/{}/releases/latest", GITHUB_REPO);
    let bytes = http_get(&url)?;
    let json = String::from_utf8(bytes).map_err(|e| format!("Invalid UTF-8 response: {}", e))?;

    let info = parse_release_json(&json)
        .ok_or_else(|| "Failed to parse release information from GitHub".to_string())?;

    if is_newer_version(&info.tag, CURRENT_VERSION) {
        Ok(Some(info))
    } else {
        Ok(None)
    }
}

fn old_exe_path(current_exe: &Path) -> PathBuf {
    let mut os = current_exe.as_os_str().to_os_string();
    os.push(".old");
    PathBuf::from(os)
}

/// Download latest release binary, staging next to the running executable when writable
/// (for an atomic same-volume swap) or falling back to the system temp directory.
pub fn download_update(url: &str) -> Result<PathBuf, String> {
    let bytes = http_get(url)?;
    if bytes.len() < 50_000 || bytes.get(0..2) != Some(b"MZ") {
        return Err("Downloaded binary appears invalid or incomplete".to_string());
    }

    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(dir) = current_exe.parent() {
            let staged = dir.join(".clickr_update.tmp");
            if std::fs::write(&staged, &bytes).is_ok() {
                return Ok(staged);
            }
        }
    }

    let temp_file = std::env::temp_dir().join("clickr_update.exe");
    std::fs::write(&temp_file, &bytes)
        .map_err(|e| format!("Failed to write update binary to temporary storage: {}", e))?;

    Ok(temp_file)
}

/// Replace a target executable path in-place with `new_binary_path`, backing up to `<exe>.old`.
fn replace_exe_in_place(current_exe: &Path, new_binary_path: &Path) -> Result<(), String> {
    let old_exe = old_exe_path(current_exe);

    // Remove any leftover .old file from an earlier update
    let _ = std::fs::remove_file(&old_exe);

    // Windows allows renaming an active, running executable in-place
    std::fs::rename(current_exe, &old_exe)
        .map_err(|e| format!("Failed to move running executable to backup: {}", e))?;

    // Move or copy the new binary into the exact same path
    if std::fs::rename(new_binary_path, current_exe).is_err() {
        // Fallback for cross-device moves (e.g. Temp on C:, Clickr on D:)
        if let Err(copy_err) = std::fs::copy(new_binary_path, current_exe) {
            // Restore original executable if copy failed
            let _ = std::fs::rename(&old_exe, current_exe);
            return Err(format!("Failed to install new executable: {}", copy_err));
        }
        let _ = std::fs::remove_file(new_binary_path);
    }

    Ok(())
}

/// Perform in-place binary swap on Windows and relaunch the exact updated executable.
pub fn apply_update(new_binary_path: &Path) -> Result<(), String> {
    let current_exe = std::env::current_exe()
        .map_err(|e| format!("Failed to locate current executable path: {}", e))?;

    if let Err(e) = replace_exe_in_place(&current_exe, new_binary_path) {
        let _ = std::fs::remove_file(new_binary_path);
        return Err(e);
    }

    // Launch the newly replaced executable at the exact same path
    let mut cmd = std::process::Command::new(&current_exe);
    if let Some(dir) = current_exe.parent() {
        cmd.current_dir(dir);
    }
    let launched = cmd.spawn();
    if launched.is_err() {
        unsafe {
            type FnShellExecuteW = unsafe extern "system" fn(
                HWND, *const u16, *const u16, *const u16, *const u16, i32,
            ) -> isize;
            let shell_lib = DynamicLib::load("shell32.dll");
            let proc = shell_lib
                .as_ref()
                .and_then(|lib| lib.proc_addr(b"ShellExecuteW\0"));
            let res = if let Some(ptr) = proc {
                let shell_exec: FnShellExecuteW = std::mem::transmute(ptr);
                with_wide_null(&current_exe.to_string_lossy(), |exe_wide| {
                    with_wide_null("open", |op_wide| {
                        shell_exec(
                            std::ptr::null_mut(),
                            op_wide,
                            exe_wide,
                            std::ptr::null(),
                            std::ptr::null(),
                            SW_SHOWNORMAL,
                        )
                    })
                })
            } else {
                0
            };
            if res as usize <= 32 {
                return Err("Failed to restart Clickr after update".to_string());
            }
        }
    }

    Ok(())
}

/// Remove any leftover `.old` backup file from a previous update.
/// Retries briefly if the previous process is still releasing its file lock during relaunch.
pub fn cleanup_old_version() {
    if let Ok(current_exe) = std::env::current_exe() {
        let old_exe = old_exe_path(&current_exe);
        if old_exe.exists() {
            for attempt in 0..6 {
                if std::fs::remove_file(&old_exe).is_ok() || !old_exe.exists() {
                    break;
                }
                if attempt < 5 {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_version() {
        assert_eq!(parse_version("v1.1.4"), Some((1, 1, 4)));
        assert_eq!(parse_version("1.1.4"), Some((1, 1, 4)));
        assert_eq!(parse_version("v0.1.0"), Some((0, 1, 0)));
        assert_eq!(parse_version("v2.0"), Some((2, 0, 0)));
    }

    #[test]
    fn test_is_newer_version() {
        assert!(is_newer_version("v1.1.4", "v0.1.0"));
        assert!(is_newer_version("v1.2.0", "v1.1.4"));
        assert!(is_newer_version("1.1.5", "1.1.4"));
        assert!(!is_newer_version("v1.1.4", "v1.1.4"));
        assert!(!is_newer_version("v1.1.3", "v1.1.4"));
        assert!(!is_newer_version("v0.1.0", "v1.1.4"));
    }

    #[test]
    fn test_extract_json_string() {
        let json = r#"{"name": "test", "tag_name": "v1.2.3", "body": "Release line 1\nLine 2"}"#;
        assert_eq!(extract_json_string(json, "name").as_deref(), Some("test"));
        assert_eq!(extract_json_string(json, "tag_name").as_deref(), Some("v1.2.3"));
        assert_eq!(extract_json_string(json, "body").as_deref(), Some("Release line 1\nLine 2"));
    }

    #[test]
    fn test_extract_asset_url() {
        let json = r#"{"assets":[{"name":"Clickr.exe","browser_download_url":"https://github.com/FabianDream/Clickr/releases/download/v1.1.4/Clickr.exe"}]}"#;
        let url = extract_asset_url(json, "Clickr.exe");
        assert_eq!(
            url.as_deref(),
            Some("https://github.com/FabianDream/Clickr/releases/download/v1.1.4/Clickr.exe")
        );
    }

    #[test]
    fn test_extract_asset_url_multibyte_utf8_boundary() {
        // Place 4-byte emoji characters right across the 1500-byte window boundary to ensure no UTF-8 panic
        let emojis = "🚀".repeat(500);
        let json = format!(
            r#"{{"body":"{}","assets":[{{"name":"clickr.exe","browser_download_url":"https://github.com/FabianDream/Clickr/releases/download/v1.1.6/Clickr.exe"}}]}}"#,
            emojis
        );
        let url = extract_asset_url(&json, "Clickr.exe");
        assert_eq!(
            url.as_deref(),
            Some("https://github.com/FabianDream/Clickr/releases/download/v1.1.6/Clickr.exe")
        );
    }

    #[test]
    fn test_cleanup_old_version() {
        // Should not panic even if no old version exists
        cleanup_old_version();
    }

    #[test]
    fn test_replace_exe_in_place_custom_path_and_name() {
        let test_dir = std::env::temp_dir().join("clickr_updater_replace_test");
        let _ = std::fs::remove_dir_all(&test_dir);
        std::fs::create_dir_all(&test_dir).unwrap();

        let custom_exe = test_dir.join("MyCustomClickr.exe");
        let staged = test_dir.join(".clickr_update.tmp");
        std::fs::write(&custom_exe, b"OLD_BINARY").unwrap();
        std::fs::write(&staged, b"NEW_BINARY").unwrap();

        replace_exe_in_place(&custom_exe, &staged).unwrap();

        assert_eq!(std::fs::read(&custom_exe).unwrap(), b"NEW_BINARY");
        assert_eq!(std::fs::read(old_exe_path(&custom_exe)).unwrap(), b"OLD_BINARY");
        assert!(!staged.exists());

        let _ = std::fs::remove_dir_all(&test_dir);
    }

    #[test]
    fn test_live_check_for_updates() {
        let update = check_for_updates().expect("GitHub update check query failed");
        // Because CARGO_PKG_VERSION is 1.1.5 and latest remote release on GitHub is v1.1.4,
        // Clickr correctly identifies that it is already up to date!
        assert!(update.is_none(), "Expected no update because 1.1.5 is newer than GitHub's latest v1.1.4");
    }
}
