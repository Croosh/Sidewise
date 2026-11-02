use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

fn default_one() -> f32 {
    1.0
}

fn default_source_bundled() -> String {
    "bundled".to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    Button,
    Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldSpec {
    #[serde(rename = "type")]
    pub field_type: FieldType,
    pub usage_page: u16,
    pub usage: u16,
}

impl FieldSpec {
    pub fn button(usage_page: u16, usage: u16) -> Self {
        Self {
            field_type: FieldType::Button,
            usage_page,
            usage,
        }
    }

    pub fn value(usage_page: u16, usage: u16) -> Self {
        Self {
            field_type: FieldType::Value,
            usage_page,
            usage,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceFields {
    pub x: FieldSpec,
    pub y: FieldSpec,
    #[serde(default = "default_contact_id")]
    pub contact_id: FieldSpec,
    #[serde(default = "default_tip_switch")]
    pub tip_switch: FieldSpec,
    #[serde(default)]
    pub confidence: Option<FieldSpec>,
    #[serde(default)]
    pub contact_count: Option<FieldSpec>,
}

fn default_contact_id() -> FieldSpec {
    FieldSpec::value(0x0D, 0x51)
}

fn default_tip_switch() -> FieldSpec {
    FieldSpec::button(0x0D, 0x42)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldDefaults {
    #[serde(default = "default_true")]
    pub confidence: bool,
    #[serde(default = "default_true")]
    pub tip_switch: bool,
}

impl Default for FieldDefaults {
    fn default() -> Self {
        Self {
            confidence: true,
            tip_switch: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct AxisOptions {
    #[serde(default)]
    pub flip_y: bool,
    #[serde(default)]
    pub flip_x: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TuningOptions {
    #[serde(default = "default_one")]
    pub edge_width_scale: f32,
    #[serde(default = "default_one")]
    pub sensitivity_scale: f32,
}

impl Default for TuningOptions {
    fn default() -> Self {
        Self {
            edge_width_scale: 1.0,
            sensitivity_scale: 1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeviceIdentity {
    pub vid: u16,
    pub pid: u16,
    pub descriptor_hash: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileMatch {
    pub vid: u16,
    pub pid: u16,
    #[serde(default)]
    pub descriptor_hash: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceProfile {
    pub id: String,
    #[serde(rename = "match")]
    pub match_device: ProfileMatch,
    pub fields: DeviceFields,
    #[serde(default)]
    pub defaults: FieldDefaults,
    #[serde(default)]
    pub axis: AxisOptions,
    #[serde(default)]
    pub tuning: TuningOptions,
    #[serde(default = "default_source_bundled")]
    pub source: String,
}

impl DeviceProfile {
    /// Validates the profile. Returns Err with description if invalid.
    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty() {
            return Err("Profile id cannot be empty".into());
        }
        if self.fields.x.usage == 0 || self.fields.x.usage_page == 0 {
            return Err("Profile missing valid X usage".into());
        }
        if self.fields.y.usage == 0 || self.fields.y.usage_page == 0 {
            return Err("Profile missing valid Y usage".into());
        }
        if self.tuning.edge_width_scale <= 0.0 || self.tuning.edge_width_scale > 5.0 {
            return Err("edge_width_scale must be between 0.0 and 5.0".into());
        }
        if self.tuning.sensitivity_scale <= 0.0 || self.tuning.sensitivity_scale > 5.0 {
            return Err("sensitivity_scale must be between 0.0 and 5.0".into());
        }
        Ok(())
    }

    /// Creates a generic fallback profile for an unknown device.
    pub fn generic(identity: &DeviceIdentity) -> Self {
        Self {
            id: format!("generic-{:04x}-{:04x}", identity.vid, identity.pid),
            match_device: ProfileMatch {
                vid: identity.vid,
                pid: identity.pid,
                descriptor_hash: Some(identity.descriptor_hash.clone()),
                name: Some(identity.name.clone()),
            },
            fields: DeviceFields {
                x: FieldSpec::value(0x01, 0x30),
                y: FieldSpec::value(0x01, 0x31),
                contact_id: FieldSpec::value(0x0D, 0x51),
                tip_switch: FieldSpec::button(0x0D, 0x42),
                confidence: Some(FieldSpec::button(0x0D, 0x47)),
                contact_count: Some(FieldSpec::value(0x0D, 0x54)),
            },
            defaults: FieldDefaults::default(),
            axis: AxisOptions::default(),
            tuning: TuningOptions::default(),
            source: "generic".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileFile {
    #[serde(default)]
    pub profile: Vec<DeviceProfile>,
}

pub struct ProfileDatabase {
    profiles: Vec<DeviceProfile>,
}

impl ProfileDatabase {
    pub fn new() -> Self {
        let mut db = Self {
            profiles: Vec::new(),
        };
        db.load_bundled();
        db
    }

    /// Loads the embedded bundled profiles (shipped as a resource in sidewise-core).
    pub fn load_bundled(&mut self) {
        let content = include_str!("profiles.toml");
        self.load_from_toml(content, "bundled");
    }

    /// Loads profiles from a TOML string, skipping malformed entries.
    pub fn load_from_toml(&mut self, content: &str, source: &str) {
        match toml::from_str::<ProfileFile>(content) {
            Ok(file) => {
                for mut prof in file.profile {
                    if prof.source.is_empty() || prof.source == "bundled" {
                        prof.source = source.to_string();
                    }
                    if let Err(err) = prof.validate() {
                        tracing::warn!("Skipping invalid profile '{}': {}", prof.id, err);
                        continue;
                    }
                    // Replace existing profile with same id if already present
                    if let Some(pos) = self.profiles.iter().position(|p| p.id == prof.id) {
                        self.profiles[pos] = prof;
                    } else {
                        self.profiles.push(prof);
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Failed to parse profile TOML ({}): {}", source, e);
            }
        }
    }

    /// Lookup order:
    /// 1. Exact match with VID, PID, and descriptor_hash
    /// 2. Match with VID and PID
    /// 3. Fallback generic profile
    pub fn lookup(&self, identity: &DeviceIdentity) -> DeviceProfile {
        // 1. Exact match with descriptor_hash
        for prof in &self.profiles {
            if prof.match_device.vid == identity.vid
                && prof.match_device.pid == identity.pid
                && prof.match_device.descriptor_hash.as_deref() == Some(&identity.descriptor_hash)
            {
                return prof.clone();
            }
        }

        // 2. Match with VID and PID
        for prof in &self.profiles {
            if prof.match_device.vid == identity.vid
                && prof.match_device.pid == identity.pid
                && prof.match_device.descriptor_hash.is_none()
            {
                return prof.clone();
            }
        }

        // 3. Generic fallback
        DeviceProfile::generic(identity)
    }

    pub fn all_profiles(&self) -> &[DeviceProfile] {
        &self.profiles
    }
}

impl Default for ProfileDatabase {
    fn default() -> Self {
        Self::new()
    }
}
