use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;
use tracing::{error, info};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIM_MODIFY, NOTIFYICONDATAW, NIF_INFO, NIIF_INFO,
};

pub const APP_VERSION: &str = "1.0.1";
const REPO_OWNER: &str = "Croosh";
const REPO_NAME: &str = "Sidewise";

static IS_CHECKING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone)]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub download_url: Option<String>,
    pub has_update: bool,
}

/// Check GitHub Releases for available updates without blocking the UI thread.
pub fn check_for_updates() -> Result<UpdateInfo, String> {
    let current = APP_VERSION;

    let releases = self_update::backends::github::ReleaseList::configure()
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .build()
        .map_err(|e| format!("Failed to configure release list: {}", e))?
        .fetch()
        .map_err(|e| format!("Failed to fetch releases: {}", e))?;

    let latest = releases
        .all()
        .first()
        .ok_or_else(|| "No releases found on GitHub".to_string())?;

    let latest_tag = latest.version().trim_start_matches('v');
    let has_update = self_update::version::bump_is_greater(current, latest_tag).unwrap_or(false);

    let download_url = latest
        .assets()
        .iter()
        .find(|a| a.name().contains("Sidewise-v") && a.name().ends_with(".zip"))
        .map(|a| a.download_url().to_string())
        .or_else(|| latest.assets().first().map(|a| a.download_url().to_string()));

    Ok(UpdateInfo {
        current_version: current.to_string(),
        latest_version: latest_tag.to_string(),
        download_url,
        has_update,
    })
}

/// Download and apply update Over-The-Air, replacing the running executable and relaunching.
pub fn apply_update() -> Result<(), String> {
    info!("Starting OTA update...");

    let releases = self_update::backends::github::ReleaseList::configure()
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .build()
        .map_err(|e| format!("Failed to configure release list: {}", e))?
        .fetch()
        .map_err(|e| format!("Failed to fetch releases: {}", e))?;

    let latest = releases
        .all()
        .first()
        .ok_or_else(|| "No releases found on GitHub".to_string())?;

    // Look for portable zip asset or setup executable
    let asset = latest
        .assets()
        .iter()
        .find(|a| a.name().ends_with(".zip") || a.name().ends_with(".exe"))
        .ok_or_else(|| "No compatible release asset found".to_string())?;

    info!("Found asset: {} at {}", asset.name(), asset.download_url());

    // Download to temp directory
    let temp_dir = tempfile::tempdir().map_err(|e| format!("Temp dir error: {}", e))?;
    let download_path = temp_dir.path().join(asset.name());

    let mut download_file = std::fs::File::create(&download_path)
        .map_err(|e| format!("Failed to create download target: {}", e))?;

    self_update::Download::from_url(asset.download_url())
        .show_download_progress(false)
        .download_to(&mut download_file)
        .map_err(|e| format!("Download error: {}", e))?;

    drop(download_file);

    let current_exe = std::env::current_exe().map_err(|e| format!("Current exe path error: {}", e))?;

    if asset.name().ends_with(".zip") {
        // Extract the executable from zip
        let zip_file = std::fs::File::open(&download_path)
            .map_err(|e| format!("Failed to open downloaded zip: {}", e))?;
        let mut archive = zip::ZipArchive::new(zip_file)
            .map_err(|e| format!("Failed to parse zip: {}", e))?;

        let mut extracted_exe = None;
        for i in 0..archive.len() {
            let mut file = archive.by_index(i).map_err(|e| format!("Zip entry error: {}", e))?;
            let name_str = file.name().map_err(|e| format!("Zip name error: {}", e))?;
            if name_str.to_lowercase().ends_with(".exe") {
                let target_path = temp_dir.path().join("new_sidewise.exe");
                let mut out = std::fs::File::create(&target_path)
                    .map_err(|e| format!("Extract file create error: {}", e))?;
                std::io::copy(&mut file, &mut out)
                    .map_err(|e| format!("Extract copy error: {}", e))?;
                extracted_exe = Some(target_path);
                break;
            }
        }

        let new_exe = extracted_exe.ok_or_else(|| "No executable found inside downloaded zip".to_string())?;

        // Safely replace current running executable using self_replace
        self_replace::self_replace(&new_exe)
            .map_err(|e| format!("Failed to replace binary: {}", e))?;
    } else {
        // Standalone exe, replace directly
        self_replace::self_replace(&download_path)
            .map_err(|e| format!("Failed to replace binary: {}", e))?;
    }

    info!("Update successfully replaced binary. Relaunching...");

    // Spawn new version
    std::process::Command::new(&current_exe)
        .spawn()
        .map_err(|e| format!("Failed to restart Sidewise: {}", e))?;

    // Exit this instance
    std::process::exit(0);
}

/// Display a native Windows notification balloon on the Sidewise tray icon.
pub unsafe fn show_tray_notification(hwnd: HWND, title: &str, message: &str) {
    let mut nid = NOTIFYICONDATAW::default();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid.uFlags = NIF_INFO;
    nid.dwInfoFlags = NIIF_INFO;

    copy_str_to_u16(&mut nid.szInfoTitle, title);
    copy_str_to_u16(&mut nid.szInfo, message);

    let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
}

fn copy_str_to_u16(dest: &mut [u16], src: &str) {
    let mut i = 0;
    for code_unit in src.encode_utf16() {
        if i >= dest.len() - 1 {
            break;
        }
        dest[i] = code_unit;
        i += 1;
    }
    if i < dest.len() {
        dest[i] = 0;
    }
}

/// Spawns a background check after startup that notifies if a new release is available.
pub fn spawn_background_update_checker(hwnd: HWND) {
    if IS_CHECKING.swap(true, Ordering::SeqCst) {
        return;
    }

    let hwnd_raw = hwnd.0 as usize;

    thread::Builder::new()
        .name("ota-update-checker".into())
        .spawn(move || {
            // Wait 12 seconds after launch to ensure zero interference with system startup
            thread::sleep(Duration::from_secs(12));

            let hwnd = HWND(hwnd_raw as *mut core::ffi::c_void);

            match check_for_updates() {
                Ok(info) if info.has_update => {
                    info!("Found newer version: v{}", info.latest_version);
                    unsafe {
                        show_tray_notification(
                            hwnd,
                            "Sidewise Update Available",
                            &format!(
                                "Version {} is available. Right-click the tray icon to update.",
                                info.latest_version
                            ),
                        );
                    }
                }
                Ok(_) => {
                    info!("Sidewise is up to date (v{})", APP_VERSION);
                }
                Err(e) => {
                    error!("Update check failed: {}", e);
                }
            }

            IS_CHECKING.store(false, Ordering::SeqCst);
        })
        .ok();
}
