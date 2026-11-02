use std::collections::HashMap;
use sidewise_core::profile::{
    DeviceFields, DeviceIdentity, DeviceProfile, FieldSpec, FieldType, ProfileDatabase,
};
use sidewise_core::types::{Contact, DeviceId, TouchFrame};
use smallvec::SmallVec;
use windows::Win32::Devices::HumanInterfaceDevice::{
    HidP_GetButtonCaps, HidP_GetCaps, HidP_GetUsageValue, HidP_GetUsagesEx, HidP_GetValueCaps,
    HIDP_BUTTON_CAPS, HIDP_CAPS, HIDP_VALUE_CAPS, PHIDP_PREPARSED_DATA, USAGE_AND_PAGE,
};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::UI::Input::{GetRawInputDeviceInfoW, RIDI_PREPARSEDDATA};

use crate::device_id::query_device_identity;

// HID Usage Pages and Usages for Precision Touchpad
const USAGE_PAGE_GENERIC_DESKTOP: u16 = 0x01;
const USAGE_X: u16 = 0x30;
const USAGE_Y: u16 = 0x31;

const USAGE_PAGE_DIGITIZER: u16 = 0x0D;
const USAGE_CONTACT_ID: u16 = 0x51;
const USAGE_TIP_SWITCH: u16 = 0x42;
const USAGE_CONFIDENCE: u16 = 0x47;
const USAGE_CONTACT_COUNT: u16 = 0x54;

unsafe fn get_button_state_ex(
    pp_handle: PHIDP_PREPARSED_DATA,
    link_col: u16,
    target_page: u16,
    target_usage: u16,
    raw_report: &[u8],
) -> bool {
    let mut btn_list = vec![USAGE_AND_PAGE::default(); 64];
    let mut btn_len = btn_list.len() as u32;
    let hr = HidP_GetUsagesEx(
        windows::Win32::Devices::HumanInterfaceDevice::HidP_Input,
        link_col,
        btn_list.as_mut_ptr(),
        &mut btn_len,
        pp_handle,
        raw_report,
    );
    if hr.is_err() {
        return false;
    }
    btn_list[..btn_len as usize]
        .iter()
        .any(|b| b.UsagePage == target_page && b.Usage == target_usage)
}

#[derive(Debug, Clone)]
pub struct AxisRange {
    pub min: i32,
    pub max: i32,
}

impl AxisRange {
    pub fn normalize(&self, raw: u32) -> f32 {
        let span = (self.max - self.min).max(1) as f32;
        let val = (raw as i32 - self.min) as f32;
        (val / span).clamp(0.0, 1.0)
    }
}

pub struct DeviceHidInfo {
    pub identity: DeviceIdentity,
    pub profile: DeviceProfile,
    pub preparsed_data: Vec<u8>,
    pub caps: HIDP_CAPS,
    pub x_range: AxisRange,
    pub y_range: AxisRange,
}

pub struct HidParser {
    devices: HashMap<isize, DeviceHidInfo>,
    profile_db: ProfileDatabase,
}

impl Default for HidParser {
    fn default() -> Self {
        Self::new()
    }
}

impl HidParser {
    pub fn new() -> Self {
        let mut db = ProfileDatabase::new();
        // Also load user custom profiles from disk if present (%APPDATA%\Sidewise\profiles)
        if let Ok(appdata) = std::env::var("APPDATA") {
            let prof_dir = std::path::PathBuf::from(appdata).join("Sidewise").join("profiles");
            if let Ok(entries) = std::fs::read_dir(prof_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|s| s.to_str()) == Some("toml") {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            db.load_from_toml(&content, "user");
                        }
                    }
                }
            }
        }

        Self {
            devices: HashMap::new(),
            profile_db: db,
        }
    }

    pub fn handle_device_change(&mut self, hdevice: HANDLE, is_arrival: bool) {
        let key = hdevice.0 as isize;
        if is_arrival {
            tracing::info!("Raw input device arrival: {:?}", hdevice);
            let _ = self.get_or_register_device(hdevice);
        } else {
            tracing::info!("Raw input device removal: {:?}", hdevice);
            self.devices.remove(&key);
        }
    }

    pub fn get_device_info(&self, hdevice: HANDLE) -> Option<&DeviceHidInfo> {
        self.devices.get(&(hdevice.0 as isize))
    }

    pub fn all_registered_devices(&self) -> Vec<(&DeviceIdentity, &DeviceProfile)> {
        self.devices.values().map(|d| (&d.identity, &d.profile)).collect()
    }

    pub fn get_or_register_device(&mut self, hdevice: HANDLE) -> Option<&DeviceHidInfo> {
        let key = hdevice.0 as isize;
        if self.devices.contains_key(&key) {
            return self.devices.get(&key);
        }

        unsafe {
            let mut data_size = 0u32;
            let _ = GetRawInputDeviceInfoW(
                hdevice,
                RIDI_PREPARSEDDATA,
                None,
                &mut data_size,
            );

            if data_size == 0 {
                return None;
            }

            let mut preparsed = vec![0u8; data_size as usize];
            let res = GetRawInputDeviceInfoW(
                hdevice,
                RIDI_PREPARSEDDATA,
                Some(preparsed.as_mut_ptr() as *mut _),
                &mut data_size,
            );

            if res == u32::MAX {
                return None;
            }

            let identity = query_device_identity(hdevice, &preparsed);
            tracing::info!(
                "Discovered touchpad: name='{}', VID={:04X}, PID={:04X}, hash={}",
                identity.name,
                identity.vid,
                identity.pid,
                identity.descriptor_hash
            );

            let mut caps = HIDP_CAPS::default();
            let pp_handle = PHIDP_PREPARSED_DATA(preparsed.as_ptr() as isize);
            if HidP_GetCaps(pp_handle, &mut caps).is_err() {
                return None;
            }

            let mut val_caps = vec![HIDP_VALUE_CAPS::default(); caps.NumberInputValueCaps as usize];
            let mut val_caps_len = caps.NumberInputValueCaps;
            let _ = HidP_GetValueCaps(
                windows::Win32::Devices::HumanInterfaceDevice::HidP_Input,
                val_caps.as_mut_ptr(),
                &mut val_caps_len,
                pp_handle,
            );

            let mut btn_caps = vec![HIDP_BUTTON_CAPS::default(); caps.NumberInputButtonCaps as usize];
            let mut btn_caps_len = caps.NumberInputButtonCaps;
            let _ = HidP_GetButtonCaps(
                windows::Win32::Devices::HumanInterfaceDevice::HidP_Input,
                btn_caps.as_mut_ptr(),
                &mut btn_caps_len,
                pp_handle,
            );

            let val_slice = &val_caps[..val_caps_len as usize];
            let btn_slice = &btn_caps[..btn_caps_len as usize];

            let mut x_range = AxisRange { min: 0, max: 4096 };
            let mut y_range = AxisRange { min: 0, max: 4096 };

            for vc in val_slice {
                if vc.UsagePage == USAGE_PAGE_GENERIC_DESKTOP {
                    if vc.Anonymous.NotRange.Usage == USAGE_X {
                        x_range = AxisRange {
                            min: vc.LogicalMin,
                            max: vc.LogicalMax,
                        };
                    } else if vc.Anonymous.NotRange.Usage == USAGE_Y {
                        y_range = AxisRange {
                            min: vc.LogicalMin,
                            max: vc.LogicalMax,
                        };
                    }
                }
            }

            // Look up device profile in database
            let mut profile = self.profile_db.lookup(&identity);

            // If the profile is generic (no custom or bundled profile matched),
            // build the field types strictly from the report descriptor caps tables:
            // value caps go through HidP_GetUsageValue, button caps through HidP_GetUsagesEx.
            // Never pick the reader from the usage name.
            if profile.source == "generic" {
                let x_is_btn = find_in_btn_caps(btn_slice, USAGE_PAGE_GENERIC_DESKTOP, USAGE_X);
                let y_is_btn = find_in_btn_caps(btn_slice, USAGE_PAGE_GENERIC_DESKTOP, USAGE_Y);
                let cid_is_btn = find_in_btn_caps(btn_slice, USAGE_PAGE_DIGITIZER, USAGE_CONTACT_ID);
                let tip_is_btn = find_in_btn_caps(btn_slice, USAGE_PAGE_DIGITIZER, USAGE_TIP_SWITCH);
                let tip_is_val = find_in_val_caps(val_slice, USAGE_PAGE_DIGITIZER, USAGE_TIP_SWITCH);

                let has_conf_btn = find_in_btn_caps(btn_slice, USAGE_PAGE_DIGITIZER, USAGE_CONFIDENCE);
                let has_conf_val = find_in_val_caps(val_slice, USAGE_PAGE_DIGITIZER, USAGE_CONFIDENCE);

                let cnt_is_btn = find_in_btn_caps(btn_slice, USAGE_PAGE_DIGITIZER, USAGE_CONTACT_COUNT);
                let cnt_is_val = find_in_val_caps(val_slice, USAGE_PAGE_DIGITIZER, USAGE_CONTACT_COUNT);

                profile.fields = DeviceFields {
                    x: if x_is_btn { FieldSpec::button(0x01, 0x30) } else { FieldSpec::value(0x01, 0x30) },
                    y: if y_is_btn { FieldSpec::button(0x01, 0x31) } else { FieldSpec::value(0x01, 0x31) },
                    contact_id: if cid_is_btn { FieldSpec::button(0x0D, 0x51) } else { FieldSpec::value(0x0D, 0x51) },
                    tip_switch: if tip_is_btn {
                        FieldSpec::button(0x0D, 0x42)
                    } else if tip_is_val {
                        FieldSpec::value(0x0D, 0x42)
                    } else {
                        tracing::info!("Device descriptor missing Tip Switch; using default tip=true");
                        FieldSpec::button(0x0D, 0x42)
                    },
                    confidence: if has_conf_btn {
                        Some(FieldSpec::button(0x0D, 0x47))
                    } else if has_conf_val {
                        Some(FieldSpec::value(0x0D, 0x47))
                    } else {
                        tracing::info!("Device descriptor missing Confidence field; using default confident=true");
                        None
                    },
                    contact_count: if cnt_is_val {
                        Some(FieldSpec::value(0x0D, 0x54))
                    } else if cnt_is_btn {
                        Some(FieldSpec::button(0x0D, 0x54))
                    } else {
                        None
                    },
                };
            }

            tracing::info!(
                "Configured profile for device: id='{}', source='{}', confidence_mode={:?}",
                profile.id,
                profile.source,
                profile.fields.confidence
            );

            let info = DeviceHidInfo {
                identity,
                profile,
                preparsed_data: preparsed,
                caps,
                x_range,
                y_range,
            };

            self.devices.insert(key, info);
            self.devices.get(&key)
        }
    }

    pub fn parse_report(
        &mut self,
        hdevice: HANDLE,
        raw_report: &[u8],
        t_ms: u64,
    ) -> Option<TouchFrame> {
        let dev_info = self.get_or_register_device(hdevice)?;
        let pp_handle = PHIDP_PREPARSED_DATA(dev_info.preparsed_data.as_ptr() as isize);
        let profile = &dev_info.profile;

        unsafe {
            let mut contact_count = 0u32;
            if let Some(ref cnt_spec) = profile.fields.contact_count {
                if cnt_spec.field_type == FieldType::Button {
                    let pressed = get_button_state_ex(
                        pp_handle,
                        0,
                        cnt_spec.usage_page,
                        cnt_spec.usage,
                        raw_report,
                    );
                    contact_count = if pressed { 1 } else { 0 };
                } else {
                    let _ = HidP_GetUsageValue(
                        windows::Win32::Devices::HumanInterfaceDevice::HidP_Input,
                        cnt_spec.usage_page,
                        0,
                        cnt_spec.usage,
                        &mut contact_count,
                        pp_handle,
                        raw_report,
                    );
                }
            }

            let count = (contact_count as usize).clamp(1, 5);
            let mut contacts = SmallVec::new();

            for link_col in 0..count as u16 {
                // 1. Contact ID
                let mut cid = 0u32;
                if profile.fields.contact_id.field_type == FieldType::Button {
                    let pressed = get_button_state_ex(
                        pp_handle,
                        link_col,
                        profile.fields.contact_id.usage_page,
                        profile.fields.contact_id.usage,
                        raw_report,
                    );
                    cid = if pressed { 1 } else { 0 };
                } else {
                    let _ = HidP_GetUsageValue(
                        windows::Win32::Devices::HumanInterfaceDevice::HidP_Input,
                        profile.fields.contact_id.usage_page,
                        link_col,
                        profile.fields.contact_id.usage,
                        &mut cid,
                        pp_handle,
                        raw_report,
                    );
                }

                // 2. X coordinate
                let mut raw_x = 0u32;
                let _ = HidP_GetUsageValue(
                    windows::Win32::Devices::HumanInterfaceDevice::HidP_Input,
                    profile.fields.x.usage_page,
                    link_col,
                    profile.fields.x.usage,
                    &mut raw_x,
                    pp_handle,
                    raw_report,
                );

                // 3. Y coordinate
                let mut raw_y = 0u32;
                let _ = HidP_GetUsageValue(
                    windows::Win32::Devices::HumanInterfaceDevice::HidP_Input,
                    profile.fields.y.usage_page,
                    link_col,
                    profile.fields.y.usage,
                    &mut raw_y,
                    pp_handle,
                    raw_report,
                );

                // 4. Tip Switch
                let tip = if profile.fields.tip_switch.field_type == FieldType::Button {
                    get_button_state_ex(
                        pp_handle,
                        link_col,
                        profile.fields.tip_switch.usage_page,
                        profile.fields.tip_switch.usage,
                        raw_report,
                    )
                } else {
                    let mut val = 0u32;
                    let _ = HidP_GetUsageValue(
                        windows::Win32::Devices::HumanInterfaceDevice::HidP_Input,
                        profile.fields.tip_switch.usage_page,
                        link_col,
                        profile.fields.tip_switch.usage,
                        &mut val,
                        pp_handle,
                        raw_report,
                    );
                    val != 0
                };

                // 5. Confidence
                let confident = match profile.fields.confidence {
                    Some(ref conf_spec) => {
                        if conf_spec.field_type == FieldType::Button {
                            get_button_state_ex(
                                pp_handle,
                                link_col,
                                conf_spec.usage_page,
                                conf_spec.usage,
                                raw_report,
                            )
                        } else {
                            let mut val = 0u32;
                            let _ = HidP_GetUsageValue(
                                windows::Win32::Devices::HumanInterfaceDevice::HidP_Input,
                                conf_spec.usage_page,
                                link_col,
                                conf_spec.usage,
                                &mut val,
                                pp_handle,
                                raw_report,
                            );
                            val != 0
                        }
                    }
                    None => profile.defaults.confidence,
                };

                let mut x = dev_info.x_range.normalize(raw_x);
                let mut y = dev_info.y_range.normalize(raw_y);

                if profile.axis.flip_x {
                    x = (1.0 - x).clamp(0.0, 1.0);
                }
                if profile.axis.flip_y {
                    y = (1.0 - y).clamp(0.0, 1.0);
                }

                contacts.push(Contact {
                    id: cid,
                    x,
                    y,
                    tip,
                    confident,
                });
            }

            Some(TouchFrame {
                t_ms,
                device: DeviceId(hdevice.0 as u64),
                contacts,
            })
        }
    }
}

unsafe fn find_in_val_caps(val_caps: &[HIDP_VALUE_CAPS], page: u16, usage: u16) -> bool {
    val_caps.iter().any(|vc| {
        vc.UsagePage == page && {
            if vc.IsRange.as_bool() {
                (vc.Anonymous.Range.UsageMin..=vc.Anonymous.Range.UsageMax).contains(&usage)
            } else {
                vc.Anonymous.NotRange.Usage == usage
            }
        }
    })
}

unsafe fn find_in_btn_caps(btn_caps: &[HIDP_BUTTON_CAPS], page: u16, usage: u16) -> bool {
    btn_caps.iter().any(|bc| {
        bc.UsagePage == page && {
            if bc.IsRange.as_bool() {
                (bc.Anonymous.Range.UsageMin..=bc.Anonymous.Range.UsageMax).contains(&usage)
            } else {
                bc.Anonymous.NotRange.Usage == usage
            }
        }
    })
}
