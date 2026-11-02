use serde::{Deserialize, Serialize};
use crate::types::{Appearance, BrightnessTarget, HapticsStrength, ModifierRequirement};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_swap_sides")]
    pub swap_sides: bool,

    #[serde(default = "default_bottom_quarter_only")]
    pub bottom_quarter_only: bool,

    #[serde(default)]
    pub require_modifier: ModifierRequirement,

    #[serde(default = "default_freeze_cursor")]
    pub freeze_cursor: bool,

    #[serde(default = "default_fine_control")]
    pub fine_control: bool,

    #[serde(default)]
    pub haptics: HapticsStrength,

    #[serde(default = "default_quick_mute")]
    pub quick_mute: bool,

    #[serde(default = "default_media_strip")]
    pub media_strip: bool,

    #[serde(default = "default_tap_to_play_pause")]
    pub tap_to_play_pause: bool,

    #[serde(default = "default_sensitivity")]
    pub sensitivity: f32,

    #[serde(default = "default_edge_width")]
    pub edge_width: f32,

    #[serde(default)]
    pub appearance: Appearance,

    #[serde(default = "default_open_at_login")]
    pub open_at_login: bool,

    #[serde(default)]
    pub brightness_target: BrightnessTarget,
}

fn default_swap_sides() -> bool { false }
fn default_bottom_quarter_only() -> bool { false }
fn default_freeze_cursor() -> bool { true }
fn default_fine_control() -> bool { true }
fn default_quick_mute() -> bool { true }
fn default_media_strip() -> bool { true }
fn default_tap_to_play_pause() -> bool { false }
fn default_sensitivity() -> f32 { 1.0 }
fn default_edge_width() -> f32 { 0.08 }
fn default_open_at_login() -> bool { true }

impl Default for Config {
    fn default() -> Self {
        Self {
            swap_sides: default_swap_sides(),
            bottom_quarter_only: default_bottom_quarter_only(),
            require_modifier: ModifierRequirement::None,
            freeze_cursor: default_freeze_cursor(),
            fine_control: default_fine_control(),
            haptics: HapticsStrength::Medium,
            quick_mute: default_quick_mute(),
            media_strip: default_media_strip(),
            tap_to_play_pause: default_tap_to_play_pause(),
            sensitivity: default_sensitivity(),
            edge_width: default_edge_width(),
            appearance: Appearance::System,
            open_at_login: default_open_at_login(),
            brightness_target: BrightnessTarget::CursorMonitor,
        }
    }
}

impl Config {
    pub fn sanitized(mut self) -> Self {
        self.sensitivity = self.sensitivity.clamp(0.5, 2.0);
        self.edge_width = self.edge_width.clamp(0.04, 0.20);
        self
    }

    pub fn from_toml(content: &str) -> Result<Self, toml::de::Error> {
        let config: Config = toml::from_str(content)?;
        Ok(config.sanitized())
    }

    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let cfg = Config::default();
        assert!(!cfg.swap_sides);
        assert!(!cfg.bottom_quarter_only);
        assert_eq!(cfg.require_modifier, ModifierRequirement::None);
        assert!(cfg.freeze_cursor);
        assert!(cfg.fine_control);
        assert_eq!(cfg.haptics, HapticsStrength::Medium);
        assert!(cfg.quick_mute);
        assert!(cfg.media_strip);
        assert!(!cfg.tap_to_play_pause);
        assert_eq!(cfg.sensitivity, 1.0);
        assert_eq!(cfg.edge_width, 0.08);
    }

    #[test]
    fn test_serialization_roundtrip() {
        let cfg = Config::default();
        let toml_str = cfg.to_toml().expect("failed to serialize");
        let parsed = Config::from_toml(&toml_str).expect("failed to deserialize");
        assert_eq!(cfg, parsed);
    }

    #[test]
    fn test_sanitization() {
        let cfg = Config {
            sensitivity: 10.0,
            edge_width: 0.01,
            ..Default::default()
        };
        let sanitized = cfg.sanitized();
        assert_eq!(sanitized.sensitivity, 2.0);
        assert_eq!(sanitized.edge_width, 0.04);
    }
}
