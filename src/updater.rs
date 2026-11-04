use std::path::{Path, PathBuf};
use std::ptr::null;
use windows_sys::Win32::Networking::WinInet::*;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

pub const GITHUB_REPO: &str = "FabianDream/Clickr";
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
    pub tag: String,
    pub version: String,
    pub download_url: String,
    pub release_notes: String,
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

fn to_wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Download contents of a URL over HTTPS using native Windows WinINet.
/// Transparently handles system proxy, certificates, and HTTPS redirects across domains.
pub fn http_get(url: &str) -> Result<Vec<u8>, String> {
    unsafe {
        let user_agent = to_wide_null("Clickr-AutoUpdater");
        let h_internet = InternetOpenW(
            user_agent.as_ptr(),
            INTERNET_OPEN_TYPE_PRECONFIG,
            null(),
            null(),
            0,
        );
        if h_internet.is_null() {
            return Err("Failed to initialize Windows network subsystem (WinINet)".to_string());
        }

        let url_wide = to_wide_null(url);
        let headers = to_wide_null("Accept: */*\r\n");
        let flags = INTERNET_FLAG_RELOAD
            | INTERNET_FLAG_SECURE
            | INTERNET_FLAG_PRAGMA_NOCACHE
            | INTERNET_FLAG_NO_CACHE_WRITE;

        let h_url = InternetOpenUrlW(
            h_internet,
            url_wide.as_ptr(),
            headers.as_ptr(),
            headers.len() as u32 - 1,
            flags,
            0,
        );

        if h_url.is_null() {
            InternetCloseHandle(h_internet);
            return Err("Failed to establish connection to GitHub".to_string());
        }

        let mut status_code: u32 = 0;
        let mut status_len = std::mem::size_of::<u32>() as u32;
        let mut index = 0u32;
        let query_flags = HTTP_QUERY_STATUS_CODE | HTTP_QUERY_FLAG_NUMBER;
        let query_res = HttpQueryInfoW(
            h_url,
            query_flags,
            &mut status_code as *mut u32 as *mut std::ffi::c_void,
            &mut status_len,
            &mut index,
        );
        if query_res != 0 && status_code != 200 {
            InternetCloseHandle(h_url);
            InternetCloseHandle(h_internet);
            return Err(format!("GitHub returned HTTP status {}", status_code));
        }

        let mut response = Vec::new();
        let mut buffer = [0u8; 16384];
        let mut bytes_read: u32 = 0;

        loop {
            let ok = InternetReadFile(
                h_url,
                buffer.as_mut_ptr() as *mut _,
                buffer.len() as u32,
                &mut bytes_read,
            );
            if ok == 0 || bytes_read == 0 {
                break;
            }
            response.extend_from_slice(&buffer[..bytes_read as usize]);
        }

        InternetCloseHandle(h_url);
        InternetCloseHandle(h_internet);

        Ok(response)
    }
}

/// Extract string value for a given key in flat/nested JSON without pulling in heavy parser crates.
pub fn extract_json_string(json: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{}\"", key);
    let mut search_idx = 0;
    while let Some(pos) = json[search_idx..].find(&pattern) {
        let key_pos = search_idx + pos + pattern.len();
        if let Some(colon_offset) = json[key_pos..].find(':') {
            let after_colon = key_pos + colon_offset + 1;
            let trimmed = json[after_colon..].trim_start();
            if trimmed.starts_with('"') {
                let str_content = &trimmed[1..];
                let mut result = String::new();
                let mut chars = str_content.chars();
                let mut escaped = false;
                while let Some(c) = chars.next() {
                    if escaped {
                        match c {
                            'n' => result.push('\n'),
                            'r' => result.push('\r'),
                            't' => result.push('\t'),
                            '\\' => result.push('\\'),
                            '"' => result.push('"'),
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
        search_idx = key_pos;
    }
    None
}

/// Find the browser download URL for an asset matching target_name.
pub fn extract_asset_url(json: &str, target_name: &str) -> Option<String> {
    let lower_json = json.to_ascii_lowercase();
    let lower_target = target_name.to_ascii_lowercase();
    let mut search_idx = 0;
    while let Some(pos) = lower_json[search_idx..].find(&lower_target) {
        let abs_pos = search_idx + pos;
        let window_start = abs_pos.saturating_sub(1500);
        let window_end = std::cmp::min(json.len(), abs_pos + 1500);
        let window = &json[window_start..window_end];
        if let Some(url) = extract_json_string(window, "browser_download_url") {
            if url.starts_with("https://") {
                return Some(url);
            }
        }
        search_idx = abs_pos + lower_target.len();
    }
    None
}

/// Parse release metadata from GitHub API response.
pub fn parse_release_json(json: &str) -> Option<UpdateInfo> {
    let tag = extract_json_string(json, "tag_name")?;
    let version = tag.trim_start_matches(|c| c == 'v' || c == 'V').to_string();
    let download_url = extract_asset_url(json, "Clickr.exe")
        .or_else(|| extract_asset_url(json, ".exe"))?;
    let release_notes = extract_json_string(json, "body").unwrap_or_default();

    Some(UpdateInfo {
        tag,
        version,
        download_url,
        release_notes,
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

/// Download latest release binary to a temporary file.
pub fn download_update(url: &str) -> Result<PathBuf, String> {
    let bytes = http_get(url)?;
    if bytes.len() < 50_000 || bytes.get(0..2) != Some(b"MZ") {
        return Err("Downloaded binary appears invalid or incomplete".to_string());
    }

    let temp_file = std::env::temp_dir().join("clickr_update.exe");
    std::fs::write(&temp_file, bytes)
        .map_err(|e| format!("Failed to write update binary to temporary storage: {}", e))?;

    Ok(temp_file)
}

/// Perform in-place binary swap on Windows and relaunch the updated executable.
pub fn apply_update(new_binary_path: &Path) -> Result<(), String> {
    let current_exe = std::env::current_exe()
        .map_err(|e| format!("Failed to locate current executable path: {}", e))?;
    let old_exe = current_exe.with_extension("exe.old");

    // Remove any leftover .old file from an earlier update
    let _ = std::fs::remove_file(&old_exe);

    // Windows allows renaming an active, running executable
    std::fs::rename(&current_exe, &old_exe)
        .map_err(|e| format!("Failed to move running executable to backup: {}", e))?;

    // Move or copy the new binary into the active location
    if let Err(_) = std::fs::rename(new_binary_path, &current_exe) {
        // Fallback for cross-device moves (e.g. Temp on C:, Clickr on D:)
        if let Err(copy_err) = std::fs::copy(new_binary_path, &current_exe) {
            // Restore original executable if copy failed
            let _ = std::fs::rename(&old_exe, &current_exe);
            return Err(format!("Failed to install new executable: {}", copy_err));
        }
        let _ = std::fs::remove_file(new_binary_path);
    }

    // Launch the new version
    let launched = std::process::Command::new(&current_exe).spawn();
    if launched.is_err() {
        unsafe {
            let exe_wide = to_wide_null(&current_exe.to_string_lossy());
            let op_wide = to_wide_null("open");
            let res = ShellExecuteW(
                std::ptr::null_mut(),
                op_wide.as_ptr(),
                exe_wide.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            );
            if res as usize <= 32 {
                return Err("Failed to restart Clickr after update".to_string());
            }
        }
    }

    Ok(())
}

/// Remove any leftover `.exe.old` backup file from a previous update.
pub fn cleanup_old_version() {
    if let Ok(current_exe) = std::env::current_exe() {
        let old_exe = current_exe.with_extension("exe.old");
        if old_exe.exists() {
            let _ = std::fs::remove_file(&old_exe);
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
    fn test_cleanup_old_version() {
        // Should not panic even if no old version exists
        cleanup_old_version();
    }

    #[test]
    fn test_live_check_for_updates() {
        let update = check_for_updates().expect("GitHub update check query failed");
        // Because CARGO_PKG_VERSION is 1.1.5 and latest remote release on GitHub is v1.1.4,
        // Clickr correctly identifies that it is already up to date!
        assert!(update.is_none(), "Expected no update because 1.1.5 is newer than GitHub's latest v1.1.4");
    }
}
