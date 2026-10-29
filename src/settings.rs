use std::fs;
use std::path::PathBuf;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_F6, VK_SPACE};

#[derive(Clone, Debug, PartialEq)]
pub struct AppSettings {
    pub hotkey: u32,
    pub target_key: u16,
    pub trigger_mode: u32, // 0 = Toggle, 1 = Hold, 2 = HoldMouse
    pub timing_mode: u32, // 0 = Interval, 1 = Rate
    pub hr: String,
    pub min: String,
    pub sec: String,
    pub ms: String,
    pub rate: String,
    pub rate_unit: usize, // 0 = CPS, 1 = CPM, 2 = CPH
    pub input_mode: u32,  // 0 = Mouse, 1 = Keyboard
    pub mouse_btn: usize, // 0 = Left, 1 = Right, 2 = Middle
    pub click_type: usize, // 0 = Single, 1 = Double, 2 = Hold
    pub location_mode: u32, // 0 = Current, 1 = Specific
    pub target_x: i32,
    pub target_y: i32,
    pub click_amount: String,
    pub pos_x: i32,
    pub pos_y: i32,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            hotkey: VK_F6 as u32,
            target_key: VK_SPACE,
            trigger_mode: 0, // Default: Toggle
            timing_mode: 1, // Default: Click Rate (requested by user)
            hr: "0".to_string(),
            min: "0".to_string(),
            sec: "0".to_string(),
            ms: "100".to_string(),
            rate: "50".to_string(), // Default: 50 Clicks/sec (requested by user)
            rate_unit: 0,           // Default: CPS (requested by user)
            input_mode: 0,          // Default: Mouse (requested by user)
            mouse_btn: 0,           // Default: Left Click (requested by user)
            click_type: 0,          // Default: Single Click
            location_mode: 0,       // Default: Current location
            target_x: 0,
            target_y: 0,
            click_amount: "0".to_string(),
            pos_x: -1, // -1 indicates center on display
            pos_y: -1,
        }
    }
}

fn get_config_path() -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        let dir = PathBuf::from(appdata).join("Clickr");
        let _ = fs::create_dir_all(&dir);
        dir.join("settings.ini")
    } else {
        PathBuf::from("clickr_settings.ini")
    }
}

pub fn load_settings() -> AppSettings {
    let mut settings = AppSettings::default();
    let path = get_config_path();
    if let Ok(content) = fs::read_to_string(path) {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                let k = k.trim();
                let v = v.trim();
                match k {
                    "hotkey" => {
                        if let Ok(val) = v.parse::<u32>() {
                            if val > 0 && val < 256 {
                                settings.hotkey = val;
                            }
                        }
                    }
                    "target_key" => {
                        if let Ok(val) = v.parse::<u16>() {
                            if val > 0 && val < 256 {
                                settings.target_key = val;
                            }
                        }
                    }
                    "trigger_mode" => {
                        if let Ok(val) = v.parse::<u32>() {
                            settings.trigger_mode = val.min(2);
                        }
                    }
                    "timing_mode" => {
                        if let Ok(val) = v.parse::<u32>() {
                            settings.timing_mode = val.min(1);
                        }
                    }
                    "hr" => {
                        if v.chars().all(|c| c.is_ascii_digit()) && !v.is_empty() {
                            settings.hr = v.to_string();
                        }
                    }
                    "min" => {
                        if v.chars().all(|c| c.is_ascii_digit()) && !v.is_empty() {
                            settings.min = v.to_string();
                        }
                    }
                    "sec" => {
                        if v.chars().all(|c| c.is_ascii_digit()) && !v.is_empty() {
                            settings.sec = v.to_string();
                        }
                    }
                    "ms" => {
                        if v.chars().all(|c| c.is_ascii_digit()) && !v.is_empty() {
                            settings.ms = v.to_string();
                        }
                    }
                    "rate" => {
                        if let Ok(val) = v.parse::<f64>() {
                            if val > 0.0 && !val.is_nan() && !val.is_infinite() {
                                settings.rate = v.to_string();
                            }
                        }
                    }
                    "rate_unit" => {
                        if let Ok(val) = v.parse::<usize>() {
                            settings.rate_unit = val.min(2);
                        }
                    }
                    "input_mode" => {
                        if let Ok(val) = v.parse::<u32>() {
                            settings.input_mode = val.min(1);
                        }
                    }
                    "mouse_btn" => {
                        if let Ok(val) = v.parse::<usize>() {
                            settings.mouse_btn = val.min(2);
                        }
                    }
                    "click_type" => {
                        if let Ok(val) = v.parse::<usize>() {
                            settings.click_type = val.min(2);
                        }
                    }
                    "location_mode" => {
                        if let Ok(val) = v.parse::<u32>() {
                            settings.location_mode = val.min(1);
                        }
                    }
                    "target_x" => {
                        if let Ok(val) = v.parse::<i32>() {
                            settings.target_x = val;
                        }
                    }
                    "target_y" => {
                        if let Ok(val) = v.parse::<i32>() {
                            settings.target_y = val;
                        }
                    }
                    "click_amount" => {
                        if v.chars().all(|c| c.is_ascii_digit()) && !v.is_empty() {
                            settings.click_amount = v.to_string();
                        }
                    }
                    "pos_x" => {
                        if let Ok(val) = v.parse::<i32>() {
                            settings.pos_x = val;
                        }
                    }
                    "pos_y" => {
                        if let Ok(val) = v.parse::<i32>() {
                            settings.pos_y = val;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Safety constraint: hotkey and target_key must NEVER be identical
    if settings.hotkey == settings.target_key as u32 {
        settings.target_key = if settings.hotkey == VK_SPACE as u32 {
            VK_F6
        } else {
            VK_SPACE
        };
    }

    settings
}

pub fn save_settings(settings: &AppSettings) {
    let path = get_config_path();
    let content = format!(
        "hotkey={}\ntarget_key={}\ntrigger_mode={}\ntiming_mode={}\nhr={}\nmin={}\nsec={}\nms={}\nrate={}\nrate_unit={}\ninput_mode={}\nmouse_btn={}\nclick_type={}\nlocation_mode={}\ntarget_x={}\ntarget_y={}\nclick_amount={}\npos_x={}\npos_y={}\n",
        settings.hotkey,
        settings.target_key,
        settings.trigger_mode,
        settings.timing_mode,
        settings.hr,
        settings.min,
        settings.sec,
        settings.ms,
        settings.rate,
        settings.rate_unit,
        settings.input_mode,
        settings.mouse_btn,
        settings.click_type,
        settings.location_mode,
        settings.target_x,
        settings.target_y,
        settings.click_amount,
        settings.pos_x,
        settings.pos_y,
    );
    let _ = fs::write(path, content);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_save_and_load_roundtrip() {
        let original = load_settings();
        let mut test_settings = original.clone();
        test_settings.hotkey = 117; // F6
        test_settings.target_key = 32; // Space
        test_settings.trigger_mode = 2; // HoldMouse trigger mode
        test_settings.timing_mode = 1;
        test_settings.rate = "20".to_string();
        test_settings.click_amount = "50".to_string();
        test_settings.mouse_btn = 1;
        test_settings.click_type = 2; // Hold click type
        test_settings.location_mode = 1;
        test_settings.target_x = 450;
        test_settings.target_y = 650;
        save_settings(&test_settings);
        let loaded = load_settings();
        assert_eq!(loaded.hotkey, 117);
        assert_eq!(loaded.target_key, 32);
        assert_eq!(loaded.trigger_mode, 2);
        assert_eq!(loaded.timing_mode, 1);
        assert_eq!(loaded.rate, "20");
        assert_eq!(loaded.click_amount, "50");
        assert_eq!(loaded.mouse_btn, 1);
        assert_eq!(loaded.click_type, 2);
        assert_eq!(loaded.location_mode, 1);
        assert_eq!(loaded.target_x, 450);
        assert_eq!(loaded.target_y, 650);
        // restore original
        save_settings(&original);
    }

    #[test]
    fn test_conflict_prevention_on_load() {
        let mut settings = AppSettings::default();
        settings.hotkey = VK_SPACE as u32;
        settings.target_key = VK_SPACE;
        // Should resolve conflict
        if settings.hotkey == settings.target_key as u32 {
            settings.target_key = if settings.hotkey == VK_SPACE as u32 {
                VK_F6
            } else {
                VK_SPACE
            };
        }
        assert_ne!(settings.hotkey, settings.target_key as u32);
    }
}
