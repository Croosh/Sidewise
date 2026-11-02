use sidewise_core::profile::DeviceIdentity;
use sidewise_core::sha256::sha256_hex;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::UI::Input::{
    GetRawInputDeviceInfoW, RIDI_DEVICEINFO, RIDI_DEVICENAME, RID_DEVICE_INFO, RIM_TYPEHID,
};

pub fn query_device_identity(hdevice: HANDLE, preparsed_data: &[u8]) -> DeviceIdentity {
    let mut vid = 0u16;
    let mut pid = 0u16;
    let mut name = String::new();

    unsafe {
        // 1. Query VID and PID via RIDI_DEVICEINFO
        let mut dev_info = RID_DEVICE_INFO {
            cbSize: std::mem::size_of::<RID_DEVICE_INFO>() as u32,
            ..Default::default()
        };
        let mut size = std::mem::size_of::<RID_DEVICE_INFO>() as u32;
        let res = GetRawInputDeviceInfoW(
            hdevice,
            RIDI_DEVICEINFO,
            Some(&mut dev_info as *mut _ as *mut _),
            &mut size,
        );

        if res != u32::MAX && dev_info.dwType == RIM_TYPEHID {
            vid = dev_info.Anonymous.hid.dwVendorId as u16;
            pid = dev_info.Anonymous.hid.dwProductId as u16;
        }

        // 2. Query device name via RIDI_DEVICENAME
        let mut name_len = 0u32;
        let _ = GetRawInputDeviceInfoW(hdevice, RIDI_DEVICENAME, None, &mut name_len);
        if name_len > 0 {
            let mut name_buf = vec![0u16; name_len as usize];
            let copied = GetRawInputDeviceInfoW(
                hdevice,
                RIDI_DEVICENAME,
                Some(name_buf.as_mut_ptr() as *mut _),
                &mut name_len,
            );
            if copied != u32::MAX {
                let null_pos = name_buf.iter().position(|&c| c == 0).unwrap_or(name_buf.len());
                name = String::from_utf16_lossy(&name_buf[..null_pos]);
            }
        }
    }

    if name.is_empty() {
        name = format!("HID Touchpad ({:04X}:{:04X})", vid, pid);
    }

    // 3. Compute SHA-256 of preparsed data
    let descriptor_hash = sha256_hex(preparsed_data);

    DeviceIdentity {
        vid,
        pid,
        descriptor_hash,
        name,
    }
}
