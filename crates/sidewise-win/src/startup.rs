use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use windows::core::PCWSTR;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW, HKEY_CURRENT_USER,
    KEY_SET_VALUE, REG_SZ,
};

const RUN_SUBKEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const APP_NAME: &str = "Sidewise";

fn to_wide_null(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

pub struct StartupManager;

impl StartupManager {
    pub fn is_enabled() -> bool {
        let subkey = to_wide_null(RUN_SUBKEY);
        let app_name = to_wide_null(APP_NAME);
        unsafe {
            let mut hkey = windows::Win32::System::Registry::HKEY::default();
            if RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                0,
                windows::Win32::System::Registry::KEY_READ,
                &mut hkey,
            ) == ERROR_SUCCESS
            {
                let mut dsize = 0u32;
                let res = windows::Win32::System::Registry::RegQueryValueExW(
                    hkey,
                    PCWSTR(app_name.as_ptr()),
                    None,
                    None,
                    None,
                    Some(&mut dsize),
                );
                let _ = RegCloseKey(hkey);
                res == ERROR_SUCCESS
            } else {
                false
            }
        }
    }

    pub fn set_enabled(enabled: bool) -> std::result::Result<(), String> {
        let subkey = to_wide_null(RUN_SUBKEY);
        let app_name = to_wide_null(APP_NAME);
        unsafe {
            let mut hkey = windows::Win32::System::Registry::HKEY::default();
            let res = RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                0,
                KEY_SET_VALUE,
                &mut hkey,
            );
            if res != ERROR_SUCCESS {
                return Err(format!("Failed to open registry key: {res:?}"));
            }

            if enabled {
                let current_exe = std::env::current_exe().map_err(|e| e.to_string())?;
                let path_str = format!("\"{}\"", current_exe.display());
                let wide_path = to_wide_null(&path_str);
                let bytes = std::slice::from_raw_parts(
                    wide_path.as_ptr() as *const u8,
                    wide_path.len() * 2,
                );
                let set_res = RegSetValueExW(
                    hkey,
                    PCWSTR(app_name.as_ptr()),
                    0,
                    REG_SZ,
                    Some(bytes),
                );
                let _ = RegCloseKey(hkey);
                if set_res != ERROR_SUCCESS {
                    return Err(format!("Failed to set registry value: {set_res:?}"));
                }
            } else {
                let del_res = RegDeleteValueW(hkey, PCWSTR(app_name.as_ptr()));
                let _ = RegCloseKey(hkey);
                if del_res != ERROR_SUCCESS && del_res.0 != 2 {
                    // 2 is ERROR_FILE_NOT_FOUND, which is fine
                    return Err(format!("Failed to delete registry value: {del_res:?}"));
                }
            }

            Ok(())
        }
    }
}
