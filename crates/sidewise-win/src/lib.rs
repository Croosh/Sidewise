#![allow(clippy::missing_safety_doc)]

pub mod audio;
pub mod brightness;
pub mod device_id;
pub mod hid;
pub mod media;
pub mod mouse_hook;
pub mod osd;
pub mod raw_input;
pub mod single_instance;
pub mod startup;
pub mod updater;

pub use audio::WindowsVolumeControl;
pub use brightness::WindowsBrightnessControl;
pub use device_id::query_device_identity;
pub use hid::HidParser;
pub use media::WindowsMediaControl;
pub use mouse_hook::CursorFreezer;
pub use osd::OsdController;
pub use raw_input::{get_latest_device_info, RawInputListener};
pub use single_instance::SingleInstance;
pub use startup::StartupManager;
pub use updater::{
    apply_update, check_for_updates, show_tray_notification, spawn_background_update_checker,
    UpdateInfo, APP_VERSION,
};
