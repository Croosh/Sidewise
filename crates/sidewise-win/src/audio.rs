use std::sync::{Arc, Mutex};
use sidewise_core::types::VolumeControl;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    eMultimedia, eRender, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
};

#[allow(clippy::arc_with_non_send_sync)]
pub struct WindowsVolumeControl {
    volume_endpoint: Arc<Mutex<Option<IAudioEndpointVolume>>>,
}

unsafe impl Send for WindowsVolumeControl {}
unsafe impl Sync for WindowsVolumeControl {}

impl Default for WindowsVolumeControl {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowsVolumeControl {
    #[allow(clippy::arc_with_non_send_sync)]
    pub fn new() -> Self {
        let control = Self {
            volume_endpoint: Arc::new(Mutex::new(None)),
        };
        let _ = control.refresh_endpoint();
        control
    }
}

impl WindowsVolumeControl {
    fn refresh_endpoint(&self) -> windows::core::Result<()> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let device: IMMDevice =
                enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia)?;
            let endpoint: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;

            let mut guard = self.volume_endpoint.lock().unwrap();
            *guard = Some(endpoint);
            Ok(())
        }
    }
}

impl VolumeControl for WindowsVolumeControl {
    fn get(&self) -> std::result::Result<f32, String> {
        let guard = self.volume_endpoint.lock().map_err(|e| e.to_string())?;
        if let Some(ref endpoint) = *guard {
            unsafe {
                if let Ok(level) = endpoint.GetMasterVolumeLevelScalar() {
                    return Ok(level);
                }
            }
        }
        drop(guard);

        let _ = self.refresh_endpoint();
        let guard = self.volume_endpoint.lock().map_err(|e| e.to_string())?;
        if let Some(ref endpoint) = *guard {
            unsafe {
                endpoint
                    .GetMasterVolumeLevelScalar()
                    .map_err(|e| e.to_string())
            }
        } else {
            Err("No audio endpoint available".to_string())
        }
    }

    fn set(&self, v: f32) -> std::result::Result<(), String> {
        let clamped = v.clamp(0.0, 1.0);
        let guard = self.volume_endpoint.lock().map_err(|e| e.to_string())?;
        if let Some(ref endpoint) = *guard {
            unsafe {
                if endpoint.SetMasterVolumeLevelScalar(clamped, std::ptr::null()).is_ok() {
                    return Ok(());
                }
            }
        }
        drop(guard);

        let _ = self.refresh_endpoint();
        let guard = self.volume_endpoint.lock().map_err(|e| e.to_string())?;
        if let Some(ref endpoint) = *guard {
            unsafe {
                endpoint
                    .SetMasterVolumeLevelScalar(clamped, std::ptr::null())
                    .map_err(|e| e.to_string())
            }
        } else {
            Err("No audio endpoint available".to_string())
        }
    }

    fn toggle_mute(&self) -> std::result::Result<bool, String> {
        let guard = self.volume_endpoint.lock().map_err(|e| e.to_string())?;
        if let Some(ref endpoint) = *guard {
            unsafe {
                if let Ok(muted) = endpoint.GetMute() {
                    let new_mute = !muted.as_bool();
                    let _ = endpoint.SetMute(new_mute, std::ptr::null());
                    return Ok(new_mute);
                }
            }
        }
        drop(guard);

        let _ = self.refresh_endpoint();
        let guard = self.volume_endpoint.lock().map_err(|e| e.to_string())?;
        if let Some(ref endpoint) = *guard {
            unsafe {
                let muted = endpoint.GetMute().map_err(|e| e.to_string())?;
                let new_mute = !muted.as_bool();
                endpoint
                    .SetMute(new_mute, std::ptr::null())
                    .map_err(|e| e.to_string())?;
                Ok(new_mute)
            }
        } else {
            Err("No audio endpoint available".to_string())
        }
    }
}
