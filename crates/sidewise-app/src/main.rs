#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use crossbeam_channel::Receiver;
use sidewise_core::config::Config;
use sidewise_core::gesture::GestureEngine;
use sidewise_core::types::{
    Action, BrightnessControl, MediaControl, SliderKind, VolumeControl,
};
use sidewise_ui::{
    FlyoutWindow, SettingsWindow, SetupGuideWindow, TrayCallbacks, TrayManager,
};
use sidewise_win::{
    CursorFreezer, OsdController, RawInputListener, SingleInstance,
    WindowsBrightnessControl, WindowsMediaControl, WindowsVolumeControl,
};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Threading::{OpenEventW, WaitForSingleObject, EVENT_ALL_ACCESS};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, PostQuitMessage, TranslateMessage, MSG,
};

fn get_config_path() -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        let dir = PathBuf::from(appdata).join("Sidewise");
        let _ = fs::create_dir_all(&dir);
        dir.join("config.toml")
    } else {
        PathBuf::from("config.toml")
    }
}

fn load_config() -> Config {
    let path = get_config_path();
    if let Ok(content) = fs::read_to_string(&path) {
        if let Ok(cfg) = Config::from_toml(&content) {
            return cfg;
        }
    }

    let default_cfg = Config::default();
    save_config(&default_cfg);
    default_cfg
}

fn save_config(cfg: &Config) {
    let path = get_config_path();
    if let Ok(serialized) = cfg.to_toml() {
        // Atomic write: write to temp file, then rename
        let tmp_path = path.with_extension("tmp");
        if fs::write(&tmp_path, serialized).is_ok() {
            let _ = fs::rename(&tmp_path, &path);
        }
    }
}

fn main() {
    unsafe {
        // Enable Per-Monitor V2 DPI awareness so all windows render crisply at native screen resolution
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
    }

    std::panic::set_hook(Box::new(|info| {
        let _ = std::fs::write("sidewise_crash.log", format!("PANIC: {:?}", info));
    }));

    // 1. Single Instance Check
    let single_instance = match SingleInstance::try_acquire() {
        Some(instance) => instance,
        None => {
            SingleInstance::signal_existing();
            return;
        }
    };

    // 2. Load Configuration
    let config = load_config();
    let config_arc = Arc::new(Mutex::new(config.clone()));

    // 3. Initialize System Action Controllers
    let volume_control = Arc::new(WindowsVolumeControl::new());
    let brightness_control = Arc::new(WindowsBrightnessControl::new());
    let media_control = Arc::new(WindowsMediaControl::new());
    let osd = Arc::new(OsdController::new());

    // 4. Initialize Gesture Engine
    let engine = Arc::new(Mutex::new(GestureEngine::new(config.clone())));

    // 5. Initialize Flyout Window
    let config_clone = Arc::clone(&config_arc);
    let engine_clone = Arc::clone(&engine);

    let on_config_change = move |new_cfg: Config| {
        save_config(&new_cfg);
        if let Ok(mut c) = config_clone.lock() {
            *c = new_cfg.clone();
        }
        if let Ok(mut eng) = engine_clone.lock() {
            eng.update_config(new_cfg);
        }
    };

    let cfg_for_settings = Arc::clone(&config_arc);
    let on_cfg_for_settings = on_config_change.clone();
    let on_open_settings = move || {
        let current_cfg = if let Ok(c) = cfg_for_settings.lock() {
            c.clone()
        } else {
            Config::default()
        };
        SettingsWindow::open(current_cfg, on_cfg_for_settings.clone());
    };

    let on_open_setup = || {
        SetupGuideWindow::open();
    };

    let flyout = Arc::new(FlyoutWindow::new(
        config.clone(),
        on_config_change.clone(),
        on_open_setup,
    ));

    // 6. Initialize System Tray Icon
    let flyout_toggle = Arc::clone(&flyout);
    let tray_callbacks = TrayCallbacks {
        on_toggle_flyout: Box::new(move || {
            flyout_toggle.toggle();
        }),
        on_open_settings: Box::new(on_open_settings.clone()),
        on_open_setup: Box::new(move || {
            SetupGuideWindow::open();
        }),
        on_quit: Box::new(|| unsafe {
            PostQuitMessage(0);
        }),
    };

    let _tray_manager = TrayManager::new(tray_callbacks);

    // 7. Start Raw Input Listener
    let (_raw_input_listener, frame_rx) = RawInputListener::new();

    // 8. Start Gesture Engine Processing Thread
    let engine_thread_eng = Arc::clone(&engine);
    let engine_vol = Arc::clone(&volume_control);
    let engine_bright = Arc::clone(&brightness_control);
    let engine_media = Arc::clone(&media_control);
    let engine_osd = Arc::clone(&osd);
    let engine_flyout = Arc::clone(&flyout);
    let engine_cfg = Arc::clone(&config_arc);

    thread::Builder::new()
        .name("engine-worker".into())
        .spawn(move || {
            run_engine_loop(
                frame_rx,
                engine_thread_eng,
                engine_vol,
                engine_bright,
                engine_media,
                engine_osd,
                engine_flyout,
                engine_cfg,
            );
        })
        .expect("Failed to start engine worker");

    // 9. Single Instance Listener Thread (signals flyout when another instance launches)
    let flyout_signal = Arc::clone(&flyout);
    thread::Builder::new()
        .name("singleton-listener".into())
        .spawn(move || {
            let event_name = windows::core::w!("Local\\Sidewise.ShowFlyout");
            unsafe {
                if let Ok(event) = OpenEventW(EVENT_ALL_ACCESS, false, event_name) {
                    loop {
                        if WaitForSingleObject(event, windows::Win32::System::Threading::INFINITE).0 == 0 {
                            flyout_signal.show_anchored();
                        }
                    }
                }
            }
        })
        .expect("Failed to start singleton listener");

    // Check if --setup flag was provided
    if std::env::args().any(|arg| arg == "--setup" || arg == "-s") {
        SetupGuideWindow::open();
    }

    // 10. Main UI Thread Win32 Message Loop
    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, HWND::default(), 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    drop(single_instance);
}

#[allow(clippy::too_many_arguments)]
fn run_engine_loop(
    frame_rx: Receiver<(sidewise_core::types::TouchFrame, sidewise_core::types::Modifiers)>,
    engine: Arc<Mutex<GestureEngine>>,
    volume_control: Arc<WindowsVolumeControl>,
    brightness_control: Arc<WindowsBrightnessControl>,
    media_control: Arc<WindowsMediaControl>,
    osd: Arc<OsdController>,
    flyout: Arc<FlyoutWindow>,
    config: Arc<Mutex<Config>>,
) {
    while let Ok((frame, mods)) = frame_rx.recv() {
        let current_vol = volume_control.get().unwrap_or(0.5);
        let current_bright = brightness_control.get().unwrap_or(0.5);

        // Forward first contact to setup guide if open
        if let Some(first) = frame.contacts.first() {
            SetupGuideWindow::record_contact(*first);
        }

        let actions;
        let is_sliding;
        let freeze_cursor;

        {
            if let Ok(mut eng) = engine.lock() {
                actions = eng.process_frame(&frame, mods, current_vol, current_bright);
                is_sliding = eng.is_sliding();
            } else {
                continue;
            }
        }

        {
            if let Ok(c) = config.lock() {
                freeze_cursor = c.freeze_cursor;
            } else {
                freeze_cursor = true;
            }
        }

        CursorFreezer::set_sliding(freeze_cursor && is_sliding);

        for action in actions {
            match action {
                Action::SetVolume(v) => {
                    let _ = volume_control.set(v);
                    osd.show(SliderKind::Volume, v, false);
                    flyout.update_levels(v, current_bright);
                    SetupGuideWindow::record_glide(SliderKind::Volume, v);
                }
                Action::SetBrightness(v) => {
                    // No programmatic trigger exists for the native brightness OSD on
                    // modern Windows builds (the shell no longer handles the old
                    // SHELLHOOK trick), so keep the custom OSD here.
                    let _ = brightness_control.set(v);
                    osd.show(SliderKind::Brightness, v, false);
                    flyout.update_levels(current_vol, v);
                    SetupGuideWindow::record_glide(SliderKind::Brightness, v);
                }
                Action::SlideStart(kind) => {
                    let (val, muted) = match kind {
                        SliderKind::Volume => (current_vol, false),
                        SliderKind::Brightness => (current_bright, false),
                    };
                    osd.show(kind, val, muted);
                    SetupGuideWindow::record_slide_start(kind);
                }
                Action::SlideEnd(kind) => {
                    CursorFreezer::set_sliding(false);
                    SetupGuideWindow::record_slide_end(kind);
                }
                Action::ToggleMute => {
                    if let Ok(muted) = volume_control.toggle_mute() {
                        let vol = volume_control.get().unwrap_or(current_vol);
                        osd.show(SliderKind::Volume, vol, muted);
                    }
                }
                Action::MediaNext => {
                    media_control.next();
                }
                Action::MediaPrev => {
                    media_control.prev();
                }
                Action::MediaPlayPause => {
                    media_control.play_pause();
                }
                Action::HapticPulse(_strength) => {
                    // Touchpad haptics pulse when supported
                }
                Action::TouchpadDiagnostic(msg) => {
                    SetupGuideWindow::surface_diagnostic_check(&msg);
                }
            }
        }
    }
}
