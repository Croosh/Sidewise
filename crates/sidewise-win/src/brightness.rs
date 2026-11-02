use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use crossbeam_channel::{bounded, Receiver, Sender};
use sidewise_core::types::BrightnessControl;
use windows::core::{w, BSTR, VARIANT};
use windows::Win32::Devices::Display::{
    DestroyPhysicalMonitors, GetMonitorBrightness, GetPhysicalMonitorsFromHMONITOR,
    SetMonitorBrightness, PHYSICAL_MONITOR,
};
use windows::Win32::Foundation::{BOOL, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, HDC, HMONITOR,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoSetProxyBlanket, CLSCTX_INPROC_SERVER,
    COINIT_MULTITHREADED, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL,
    RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows::Win32::System::Wmi::{
    IWbemClassObject, IWbemLocator, IWbemServices, WbemLocator,
    WBEM_FLAG_FORWARD_ONLY, WBEM_FLAG_RETURN_IMMEDIATELY, WBEM_GENERIC_FLAG_TYPE,
    WBEM_INFINITE,
};

pub struct WindowsBrightnessControl {
    current_level: Arc<AtomicU32>, // 0 to 100
    sender: Sender<f32>,
}

impl Default for WindowsBrightnessControl {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowsBrightnessControl {
    pub fn new() -> Self {
        let current_level = Arc::new(AtomicU32::new(50));
        let (tx, rx): (Sender<f32>, Receiver<f32>) = bounded(10);

        let level_clone = Arc::clone(&current_level);
        // Spawn latest-value-wins worker thread to avoid blocking the engine or input thread
        thread::Builder::new()
            .name("brightness-worker".into())
            .spawn(move || {
                Self::worker_loop(rx, level_clone);
            })
            .expect("Failed to spawn brightness worker");

        Self {
            current_level,
            sender: tx,
        }
    }

    fn worker_loop(rx: Receiver<f32>, current_level: Arc<AtomicU32>) {
        // Connect to WMI on the worker thread and keep the connection open for the lifetime of the process
        let wmi_services = unsafe { connect_wmi() };

        // Read initial brightness from WMI
        if let Some(ref services) = wmi_services {
            if let Some(val) = unsafe { query_wmi_brightness(services) } {
                current_level.store((val * 100.0).round() as u32, Ordering::SeqCst);
            }
        } else if let Some(val) = Self::query_current_hardware_brightness_ddcci() {
            current_level.store((val * 100.0).round() as u32, Ordering::SeqCst);
        }

        while let Ok(mut val) = rx.recv() {
            // Drain queue so only latest value is applied
            while let Ok(newer) = rx.try_recv() {
                val = newer;
            }

            let percent = (val.clamp(0.0, 1.0) * 100.0).round() as u32;
            current_level.store(percent, Ordering::SeqCst);

            // 1. Try WMI first (Internal panels like Surface/ThinkPad/Dell laptop screens)
            let mut applied = false;
            if let Some(ref services) = wmi_services {
                if unsafe { set_wmi_brightness(services, percent).is_ok() } {
                    applied = true;
                }
            }

            // 2. If WMI was not applied (desktops/external monitors), fallback to DDC/CI
            if !applied {
                let _ = Self::apply_hardware_brightness_ddcci(percent);
            }

            // Throttle to 30ms to avoid flooding DDC/CI or WMI
            thread::sleep(Duration::from_millis(30));
        }
    }

    fn query_current_hardware_brightness_ddcci() -> Option<f32> {
        unsafe {
            struct MonitorContext {
                brightness: Option<f32>,
            }
            let mut ctx = MonitorContext { brightness: None };

            unsafe extern "system" fn enum_proc(
                hmonitor: HMONITOR,
                _hdc: HDC,
                _rect: *mut RECT,
                lparam: LPARAM,
            ) -> BOOL {
                let ctx = &mut *(lparam.0 as *mut MonitorContext);
                let mut monitors = [PHYSICAL_MONITOR::default(); 4];
                if GetPhysicalMonitorsFromHMONITOR(hmonitor, &mut monitors).is_ok() {
                    for mon in &monitors {
                        let h_phys = mon.hPhysicalMonitor;
                        if !h_phys.is_invalid() {
                            let mut min = 0u32;
                            let mut cur = 0u32;
                            let mut max = 100u32;
                            if GetMonitorBrightness(h_phys, &mut min, &mut cur, &mut max) != 0 {
                                let range = (max - min).max(1) as f32;
                                ctx.brightness = Some(((cur - min) as f32 / range).clamp(0.0, 1.0));
                                let _ = DestroyPhysicalMonitors(&monitors);
                                return BOOL(0); // Stop enum
                            }
                        }
                    }
                    let _ = DestroyPhysicalMonitors(&monitors);
                }
                BOOL(1)
            }

            let _ = EnumDisplayMonitors(
                None,
                None,
                Some(enum_proc),
                LPARAM(&mut ctx as *mut _ as isize),
            );

            ctx.brightness
        }
    }

    fn apply_hardware_brightness_ddcci(percent: u32) -> Result<(), ()> {
        unsafe {
            struct MonitorContext {
                percent: u32,
                applied: bool,
            }
            let mut ctx = MonitorContext {
                percent,
                applied: false,
            };

            unsafe extern "system" fn enum_proc(
                hmonitor: HMONITOR,
                _hdc: HDC,
                _rect: *mut RECT,
                lparam: LPARAM,
            ) -> BOOL {
                let ctx = &mut *(lparam.0 as *mut MonitorContext);
                let mut monitors = [PHYSICAL_MONITOR::default(); 4];
                if GetPhysicalMonitorsFromHMONITOR(hmonitor, &mut monitors).is_ok() {
                    for mon in &monitors {
                        let h_phys = mon.hPhysicalMonitor;
                        if !h_phys.is_invalid() {
                            let mut min = 0u32;
                            let mut cur = 0u32;
                            let mut max = 100u32;
                            if GetMonitorBrightness(h_phys, &mut min, &mut cur, &mut max) != 0 {
                                let target = min + (((max - min) as f32) * (ctx.percent as f32 / 100.0)).round() as u32;
                                let _ = SetMonitorBrightness(h_phys, target);
                                ctx.applied = true;
                            }
                        }
                    }
                    let _ = DestroyPhysicalMonitors(&monitors);
                }
                BOOL(1)
            }

            let _ = EnumDisplayMonitors(
                None,
                None,
                Some(enum_proc),
                LPARAM(&mut ctx as *mut _ as isize),
            );

            if ctx.applied {
                Ok(())
            } else {
                Err(())
            }
        }
    }
}

impl BrightnessControl for WindowsBrightnessControl {
    fn get(&self) -> std::result::Result<f32, String> {
        let val = self.current_level.load(Ordering::Relaxed) as f32 / 100.0;
        Ok(val.clamp(0.0, 1.0))
    }

    fn set(&self, v: f32) -> std::result::Result<(), String> {
        let clamped = v.clamp(0.0, 1.0);
        self.current_level.store((clamped * 100.0) as u32, Ordering::Relaxed);
        let _ = self.sender.try_send(clamped);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// WMI Brightness Backend (root\WMI -> WmiMonitorBrightness / WmiSetBrightness)
// ---------------------------------------------------------------------------

unsafe fn connect_wmi() -> Option<IWbemServices> {
    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER).ok()?;
    let namespace = BSTR::from("root\\WMI");
    let services = locator.ConnectServer(
        &namespace,
        None,
        None,
        None,
        0,
        None,
        None,
    ).ok()?;

    let _ = CoSetProxyBlanket(
        &services,
        10, // RPC_C_AUTHN_WINNT
        0,  // RPC_C_AUTHZ_NONE
        None,
        RPC_C_AUTHN_LEVEL_CALL,
        RPC_C_IMP_LEVEL_IMPERSONATE,
        None,
        EOAC_NONE,
    );

    Some(services)
}

unsafe fn query_wmi_brightness(services: &IWbemServices) -> Option<f32> {
    let query_lang = BSTR::from("WQL");
    let query = BSTR::from("SELECT CurrentBrightness FROM WmiMonitorBrightness WHERE Active = TRUE");
    let enumerator = services.ExecQuery(
        &query_lang,
        &query,
        WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
        None,
    ).ok()?;

    let mut objects = [None; 1];
    let mut returned = 0u32;
    let hr = enumerator.Next(
        WBEM_INFINITE,
        &mut objects,
        &mut returned,
    );

    if hr.is_err() || returned == 0 {
        return None;
    }

    if let Some(obj) = &objects[0] {
        let mut var = VARIANT::default();
        if obj.Get(w!("CurrentBrightness"), 0, &mut var, None, None).is_ok() {
            let val = u32::try_from(&var).unwrap_or(50);
            return Some((val as f32 / 100.0).clamp(0.0, 1.0));
        }
    }
    None
}

unsafe fn set_wmi_brightness(services: &IWbemServices, percent: u32) -> Result<(), ()> {
    let query_lang = BSTR::from("WQL");
    let query = BSTR::from("SELECT __PATH FROM WmiMonitorBrightnessMethods WHERE Active = TRUE");
    let enumerator = services.ExecQuery(
        &query_lang,
        &query,
        WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
        None,
    ).map_err(|_| ())?;

    let mut objects = [None; 1];
    let mut returned = 0u32;
    let hr = enumerator.Next(
        WBEM_INFINITE,
        &mut objects,
        &mut returned,
    );

    if hr.is_err() || returned == 0 {
        return Err(());
    }
    let instance = objects[0].as_ref().ok_or(())?;

    let mut path_var = VARIANT::default();
    instance.Get(w!("__PATH"), 0, &mut path_var, None, None).map_err(|_| ())?;
    let path_bstr = BSTR::try_from(&path_var).map_err(|_| ())?;

    let class_name = BSTR::from("WmiMonitorBrightnessMethods");
    let method_name = BSTR::from("WmiSetBrightness");

    let mut class_obj: Option<IWbemClassObject> = None;
    services.GetObject(
        &class_name,
        WBEM_GENERIC_FLAG_TYPE(0),
        None,
        Some(&mut class_obj),
        None,
    ).map_err(|_| ())?;
    let class_obj = class_obj.ok_or(())?;

    let mut in_params_def: Option<IWbemClassObject> = None;
    class_obj.GetMethod(
        w!("WmiSetBrightness"),
        0,
        &mut in_params_def,
        std::ptr::null_mut(),
    ).map_err(|_| ())?;
    let in_params_def = in_params_def.ok_or(())?;

    let in_params = in_params_def.SpawnInstance(0).map_err(|_| ())?;

    // Timeout: 1 (VT_I4 / i32 is required by WMI Automation InParams parser)
    let timeout_var = VARIANT::from(1i32);
    in_params.Put(w!("Timeout"), 0, &timeout_var as *const _, 0).map_err(|_| ())?;

    // Brightness: percent (VT_UI1 / u8)
    let bright_var = VARIANT::from(percent.clamp(0, 100) as u8);
    in_params.Put(w!("Brightness"), 0, &bright_var as *const _, 0).map_err(|_| ())?;

    services.ExecMethod(
        &path_bstr,
        &method_name,
        WBEM_GENERIC_FLAG_TYPE(0),
        None,
        &in_params,
        None,
        None,
    ).map_err(|_| ())?;

    Ok(())
}
