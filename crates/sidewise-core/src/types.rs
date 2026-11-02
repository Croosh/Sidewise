use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeviceId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Contact {
    pub id: u32,
    /// Normalized coordinate in range [0.0, 1.0], origin top-left.
    pub x: f32,
    /// Normalized coordinate in range [0.0, 1.0], origin top-left.
    pub y: f32,
    /// Contact tip switch (true if touching the surface).
    pub tip: bool,
    /// Confidence flag for palm rejection.
    pub confident: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TouchFrame {
    /// Timestamp in milliseconds.
    pub t_ms: u64,
    /// Originating device ID.
    pub device: DeviceId,
    /// List of contacts present in this frame.
    pub contacts: SmallVec<[Contact; 5]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Modifiers {
    pub alt: bool,
    pub ctrl: bool,
    pub shift: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SliderKind {
    Volume,
    Brightness,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ModifierRequirement {
    #[default]
    None,
    Alt,
    Ctrl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HapticsStrength {
    Off,
    Light,
    #[default]
    Medium,
    Strong,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BrightnessTarget {
    #[default]
    CursorMonitor,
    All,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    SetVolume(f32),
    SetBrightness(f32),
    SlideStart(SliderKind),
    SlideEnd(SliderKind),
    ToggleMute,
    MediaNext,
    MediaPrev,
    MediaPlayPause,
    HapticPulse(HapticsStrength),
    TouchpadDiagnostic(String),
}

pub trait VolumeControl: Send + Sync {
    fn get(&self) -> Result<f32, String>;
    fn set(&self, v: f32) -> Result<(), String>;
    fn toggle_mute(&self) -> Result<bool, String>;
}

pub trait BrightnessControl: Send + Sync {
    fn get(&self) -> Result<f32, String>;
    fn set(&self, v: f32) -> Result<(), String>;
}

pub trait MediaControl: Send + Sync {
    fn next(&self);
    fn prev(&self);
    fn play_pause(&self);
}
