use crate::config::Config;
use crate::types::{
    Action, Contact, DeviceId, HapticsStrength, ModifierRequirement, Modifiers, SliderKind,
    TouchFrame,
};

const STRIP_HEIGHT: f32 = 0.12;
const MUTE_CORNER_SIZE: f32 = 0.15;
const ARMING_DURATION_MS: u64 = 60;
const ARM_MAX_LATERAL_DRIFT: f32 = 0.05;
const ARM_MIN_VERTICAL_MOVE: f32 = 0.015;
const PALM_ISOLATION_DIST: f32 = 0.25;
const SMOOTHING_ALPHA: f32 = 0.35;
const TAP_MAX_DURATION_MS: u64 = 180;
const TAP_MAX_TRAVEL: f32 = 0.02;
const DOUBLE_TAP_MAX_INTERVAL_MS: u64 = 350;
const MEDIA_SWIPE_MIN_DIST: f32 = 0.15;
const MEDIA_SWIPE_MAX_TIME_MS: u64 = 400;

#[derive(Debug, Clone, Copy, PartialEq)]
enum SliderState {
    Idle,
    Armed {
        kind: SliderKind,
        contact_id: u32,
        start_x: f32,
        start_y: f32,
        start_value: f32,
        start_t_ms: u64,
        device: DeviceId,
    },
    Sliding {
        kind: SliderKind,
        contact_id: u32,
        anchor_y: f32,
        anchor_value: f32,
        smoothed_value: f32,
        last_emitted_value: f32,
        last_haptic_tick_value: f32,
        last_fine_held: bool,
        device: DeviceId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct MediaContactTracker {
    contact_id: u32,
    start_x: f32,
    start_y: f32,
    start_t_ms: u64,
    consumed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct CornerContactTracker {
    contact_id: u32,
    start_x: f32,
    start_y: f32,
    start_t_ms: u64,
}

#[derive(Debug, Clone)]
struct EdgeHealthTracker {
    contact_id: u32,
    start_t_ms: u64,
    has_emitted: bool,
}

#[derive(Debug, Clone)]
pub struct GestureEngine {
    config: Config,
    slider_state: SliderState,
    media_tracker: Option<MediaContactTracker>,
    corner_tracker: Option<CornerContactTracker>,
    last_corner_tap_t_ms: Option<u64>,
    edge_health: Option<EdgeHealthTracker>,
}

impl GestureEngine {
    pub fn new(config: Config) -> Self {
        Self {
            config: config.sanitized(),
            slider_state: SliderState::Idle,
            media_tracker: None,
            corner_tracker: None,
            last_corner_tap_t_ms: None,
            edge_health: None,
        }
    }

    pub fn update_config(&mut self, config: Config) {
        self.config = config.sanitized();
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn is_sliding(&self) -> bool {
        matches!(self.slider_state, SliderState::Sliding { .. }) || self.media_tracker.is_some()
    }

    pub fn current_slider_kind(&self) -> Option<SliderKind> {
        match self.slider_state {
            SliderState::Sliding { kind, .. } | SliderState::Armed { kind, .. } => Some(kind),
            SliderState::Idle => None,
        }
    }

    /// Process an incoming touch frame.
    /// `current_volume` and `current_brightness` are the latest known system levels (0.0 to 1.0).
    /// `modifiers` represents the currently held keyboard modifiers.
    pub fn process_frame(
        &mut self,
        frame: &TouchFrame,
        modifiers: Modifiers,
        current_volume: f32,
        current_brightness: f32,
    ) -> Vec<Action> {
        let mut actions = Vec::new();

        self.process_slider(
            frame,
            modifiers,
            current_volume,
            current_brightness,
            &mut actions,
        );

        if self.config.media_strip {
            self.process_media(frame, modifiers, &mut actions);
        } else {
            self.media_tracker = None;
        }

        if self.config.quick_mute {
            self.process_mute_corner(frame, &mut actions);
        } else {
            self.corner_tracker = None;
            self.last_corner_tap_t_ms = None;
        }

        self.process_edge_health(frame, &mut actions);

        actions
    }

    fn process_edge_health(&mut self, frame: &TouchFrame, actions: &mut Vec<Action>) {
        if self.is_sliding() {
            self.edge_health = None;
            return;
        }

        let edge_width = self.config.edge_width;
        let edge_contact = frame.contacts.iter().find(|c| {
            c.tip && (c.x <= edge_width || c.x >= 1.0 - edge_width)
        });

        match (edge_contact, &mut self.edge_health) {
            (Some(c), Some(tracker)) if tracker.contact_id == c.id => {
                if !tracker.has_emitted && frame.t_ms.saturating_sub(tracker.start_t_ms) >= 600 {
                    tracker.has_emitted = true;
                    tracing::debug!("Runtime health check: contact remained in edge zone without sliding");
                    actions.push(Action::TouchpadDiagnostic(
                        "Touchpad check: Contact held in edge zone without starting a slide. Run touchpad check in setup.".into(),
                    ));
                }
            }
            (Some(c), _) => {
                self.edge_health = Some(EdgeHealthTracker {
                    contact_id: c.id,
                    start_t_ms: frame.t_ms,
                    has_emitted: false,
                });
            }
            (None, _) => {
                self.edge_health = None;
            }
        }
    }

    fn check_modifier_requirement(&self, modifiers: Modifiers) -> bool {
        match self.config.require_modifier {
            ModifierRequirement::None => true,
            ModifierRequirement::Alt => modifiers.alt,
            ModifierRequirement::Ctrl => modifiers.ctrl,
        }
    }

    fn identify_slider_zone(&self, x: f32, y: f32) -> Option<SliderKind> {
        if self.config.bottom_quarter_only && y < 0.75 {
            return None;
        }

        let edge_width = self.config.edge_width;
        let is_left = x <= edge_width;
        let is_right = x >= 1.0 - edge_width;

        if !is_left && !is_right {
            return None;
        }

        if !self.config.swap_sides {
            if is_right {
                Some(SliderKind::Volume)
            } else {
                Some(SliderKind::Brightness)
            }
        } else {
            if is_right {
                Some(SliderKind::Brightness)
            } else {
                Some(SliderKind::Volume)
            }
        }
    }

    fn process_slider(
        &mut self,
        frame: &TouchFrame,
        modifiers: Modifiers,
        current_volume: f32,
        current_brightness: f32,
        actions: &mut Vec<Action>,
    ) {
        // Multi-finger palm rejection: if 3 or more contacts are down, cancel any active or armed slide.
        if frame.contacts.len() >= 3 {
            if let SliderState::Sliding { kind, .. } = self.slider_state {
                actions.push(Action::SlideEnd(kind));
            }
            self.slider_state = SliderState::Idle;
            return;
        }

        match self.slider_state {
            SliderState::Idle => {
                if !self.check_modifier_requirement(modifiers) {
                    return;
                }

                // Look for a confident, tip-down contact that just landed in an edge zone
                for contact in &frame.contacts {
                    if !contact.tip || !contact.confident {
                        continue;
                    }

                    if let Some(kind) = self.identify_slider_zone(contact.x, contact.y) {
                        // Palm rejection check: no other contact should be within 25% of pad width
                        let isolated = frame.contacts.iter().all(|c| {
                            c.id == contact.id || (c.x - contact.x).abs() >= PALM_ISOLATION_DIST
                        });

                        if isolated {
                            let start_value = match kind {
                                SliderKind::Volume => current_volume.clamp(0.0, 1.0),
                                SliderKind::Brightness => current_brightness.clamp(0.0, 1.0),
                            };

                            self.slider_state = SliderState::Armed {
                                kind,
                                contact_id: contact.id,
                                start_x: contact.x,
                                start_y: contact.y,
                                start_value,
                                start_t_ms: frame.t_ms,
                                device: frame.device,
                            };
                            break;
                        }
                    }
                }
            }

            SliderState::Armed {
                kind,
                contact_id,
                start_x,
                start_y,
                start_value,
                start_t_ms,
                device,
            } => {
                // If a second finger lands, cancel arming
                let active_contacts: Vec<&Contact> =
                    frame.contacts.iter().filter(|c| c.tip).collect();
                if active_contacts.len() > 1 {
                    self.slider_state = SliderState::Idle;
                    return;
                }

                // Find tracked contact
                let contact = frame.contacts.iter().find(|c| c.id == contact_id && c.tip);
                let contact = match contact {
                    Some(c) => c,
                    None => {
                        // Contact lifted before sliding was confirmed
                        self.slider_state = SliderState::Idle;
                        return;
                    }
                };

                let dx = (contact.x - start_x).abs();
                let dy = (contact.y - start_y).abs();
                let dt = frame.t_ms.saturating_sub(start_t_ms);

                // If contact drifts sideways out of the arming zone
                if dx > ARM_MAX_LATERAL_DRIFT {
                    self.slider_state = SliderState::Idle;
                    return;
                }

                // If moves vertically by > 1.5% pad height within or up to arming duration
                if dy >= ARM_MIN_VERTICAL_MOVE {
                    actions.push(Action::SlideStart(kind));
                    let fine_active =
                        self.config.fine_control && modifiers.alt && modifiers.shift;
                    self.slider_state = SliderState::Sliding {
                        kind,
                        contact_id,
                        anchor_y: start_y,
                        anchor_value: start_value,
                        smoothed_value: start_value,
                        last_emitted_value: start_value,
                        last_haptic_tick_value: start_value,
                        last_fine_held: fine_active,
                        device,
                    };
                } else if dt > ARMING_DURATION_MS * 4 && dy < ARM_MIN_VERTICAL_MOVE {
                    // Stagnant in arming zone without vertical movement: return to idle
                    self.slider_state = SliderState::Idle;
                }
            }

            SliderState::Sliding {
                kind,
                contact_id,
                ref mut anchor_y,
                ref mut anchor_value,
                ref mut smoothed_value,
                ref mut last_emitted_value,
                ref mut last_haptic_tick_value,
                ref mut last_fine_held,
                device: _,
            } => {
                let active_contacts: Vec<&Contact> =
                    frame.contacts.iter().filter(|c| c.tip).collect();
                // Slide ends if finger lifts or multiple fingers are down
                if active_contacts.len() != 1 {
                    actions.push(Action::SlideEnd(kind));
                    self.slider_state = SliderState::Idle;
                    return;
                }

                let contact = match frame.contacts.iter().find(|c| c.id == contact_id && c.tip) {
                    Some(c) => c,
                    None => {
                        actions.push(Action::SlideEnd(kind));
                        self.slider_state = SliderState::Idle;
                        return;
                    }
                };

                let fine_active = self.config.fine_control && modifiers.alt && modifiers.shift;
                // Re-anchor when fine control changes mid-slide to prevent jumps
                if fine_active != *last_fine_held {
                    *anchor_y = contact.y;
                    *anchor_value = *smoothed_value;
                    *last_fine_held = fine_active;
                }

                let zone_length = if self.config.bottom_quarter_only {
                    0.25
                } else {
                    1.0
                };
                let gain = self.config.sensitivity * if fine_active { 0.25 } else { 1.0 };

                // Sliding up (contact.y < anchor_y) raises the value
                let raw_delta = (*anchor_y - contact.y) / zone_length;
                let target_value = (*anchor_value + gain * raw_delta).clamp(0.0, 1.0);

                // Low-pass smoothing filter (one-pole, alpha = 0.35)
                *smoothed_value =
                    (1.0 - SMOOTHING_ALPHA) * (*smoothed_value) + SMOOTHING_ALPHA * target_value;
                *smoothed_value = (*smoothed_value).clamp(0.0, 1.0);

                // Quantize to 1% step
                let quantized = (*smoothed_value * 100.0).round() / 100.0;

                if (quantized - *last_emitted_value).abs() >= 0.01 {
                    *last_emitted_value = quantized;
                    match kind {
                        SliderKind::Volume => actions.push(Action::SetVolume(quantized)),
                        SliderKind::Brightness => actions.push(Action::SetBrightness(quantized)),
                    }
                }

                // Haptic feedback tick every 2%
                if (quantized - *last_haptic_tick_value).abs() >= 0.02 {
                    *last_haptic_tick_value = quantized;
                    if self.config.haptics != HapticsStrength::Off {
                        actions.push(Action::HapticPulse(self.config.haptics));
                    }
                }
            }
        }
    }

    fn process_media(
        &mut self,
        frame: &TouchFrame,
        modifiers: Modifiers,
        actions: &mut Vec<Action>,
    ) {
        // If sliding volume or brightness, disable media strip interactions
        if matches!(self.slider_state, SliderState::Sliding { .. }) {
            self.media_tracker = None;
            return;
        }

        let bottom_threshold = 1.0 - STRIP_HEIGHT;

        if let Some(mut tracker) = self.media_tracker {
            let contact = frame.contacts.iter().find(|c| c.id == tracker.contact_id);
            match contact {
                Some(c) if c.tip => {
                    let dx = c.x - tracker.start_x;
                    let dt = frame.t_ms.saturating_sub(tracker.start_t_ms);

                    // Check horizontal swipe
                    if !tracker.consumed && dt <= MEDIA_SWIPE_MAX_TIME_MS {
                        if dx >= MEDIA_SWIPE_MIN_DIST {
                            actions.push(Action::MediaNext);
                            tracker.consumed = true;
                            self.media_tracker = Some(tracker);
                        } else if dx <= -MEDIA_SWIPE_MIN_DIST {
                            actions.push(Action::MediaPrev);
                            tracker.consumed = true;
                            self.media_tracker = Some(tracker);
                        }
                    }
                }
                _ => {
                    // Contact lifted
                    let dt = frame.t_ms.saturating_sub(tracker.start_t_ms);
                    if !tracker.consumed && dt <= TAP_MAX_DURATION_MS && self.config.tap_to_play_pause {
                        actions.push(Action::MediaPlayPause);
                    }
                    self.media_tracker = None;
                }
            }
        } else {
            // Require configured modifier before arming media strip (consistent with sliders)
            if !self.check_modifier_requirement(modifiers) {
                return;
            }

            // Check for new contact in bottom strip
            for contact in &frame.contacts {
                if contact.tip && contact.confident && contact.y >= bottom_threshold {
                    self.media_tracker = Some(MediaContactTracker {
                        contact_id: contact.id,
                        start_x: contact.x,
                        start_y: contact.y,
                        start_t_ms: frame.t_ms,
                        consumed: false,
                    });
                    break;
                }
            }
        }
    }

    fn process_mute_corner(&mut self, frame: &TouchFrame, actions: &mut Vec<Action>) {
        let corner_x = 1.0 - MUTE_CORNER_SIZE;
        let corner_y = MUTE_CORNER_SIZE;

        if let Some(tracker) = self.corner_tracker {
            let contact = frame.contacts.iter().find(|c| c.id == tracker.contact_id);
            match contact {
                Some(c) if c.tip => {
                    let travel = ((c.x - tracker.start_x).powi(2) + (c.y - tracker.start_y).powi(2)).sqrt();
                    if travel > TAP_MAX_TRAVEL {
                        // Traveled too much, cancel corner tap
                        self.corner_tracker = None;
                    }
                }
                _ => {
                    // Contact lifted
                    let dt = frame.t_ms.saturating_sub(tracker.start_t_ms);
                    if dt <= TAP_MAX_DURATION_MS {
                        if let Some(last_tap_t) = self.last_corner_tap_t_ms {
                            let interval = frame.t_ms.saturating_sub(last_tap_t);
                            if interval <= DOUBLE_TAP_MAX_INTERVAL_MS {
                                actions.push(Action::ToggleMute);
                                self.last_corner_tap_t_ms = None;
                                self.corner_tracker = None;
                                return;
                            }
                        }
                        self.last_corner_tap_t_ms = Some(frame.t_ms);
                    }
                    self.corner_tracker = None;
                }
            }
        } else {
            // Check if last tap expired
            if let Some(last_tap_t) = self.last_corner_tap_t_ms {
                if frame.t_ms.saturating_sub(last_tap_t) > DOUBLE_TAP_MAX_INTERVAL_MS {
                    self.last_corner_tap_t_ms = None;
                }
            }

            for contact in &frame.contacts {
                if contact.tip && contact.confident && contact.x >= corner_x && contact.y <= corner_y {
                    self.corner_tracker = Some(CornerContactTracker {
                        contact_id: contact.id,
                        start_x: contact.x,
                        start_y: contact.y,
                        start_t_ms: frame.t_ms,
                    });
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smallvec::SmallVec;

    fn make_frame(t_ms: u64, contacts: Vec<Contact>) -> TouchFrame {
        TouchFrame {
            t_ms,
            device: DeviceId(1),
            contacts: SmallVec::from_vec(contacts),
        }
    }

    #[test]
    fn test_volume_slide_gesture() {
        let mut engine = GestureEngine::new(Config::default());
        let mods = Modifiers::default();

        // 1. Touch lands on right edge (x = 0.95, y = 0.5)
        let f1 = make_frame(100, vec![Contact { id: 1, x: 0.95, y: 0.5, tip: true, confident: true }]);
        let acts1 = engine.process_frame(&f1, mods, 0.50, 0.50);
        assert!(acts1.is_empty());
        assert!(!engine.is_sliding());

        // 2. Moves vertically by > 0.015 (y = 0.45) at t = 140ms
        let f2 = make_frame(140, vec![Contact { id: 1, x: 0.95, y: 0.45, tip: true, confident: true }]);
        let acts2 = engine.process_frame(&f2, mods, 0.50, 0.50);
        assert!(engine.is_sliding());
        assert_eq!(acts2[0], Action::SlideStart(SliderKind::Volume));

        // 3. Slides upwards (y = 0.20) at t = 200ms -> volume increases
        let f3 = make_frame(200, vec![Contact { id: 1, x: 0.95, y: 0.20, tip: true, confident: true }]);
        let acts3 = engine.process_frame(&f3, mods, 0.50, 0.50);
        assert!(!acts3.is_empty());
        let has_vol = acts3.iter().any(|a| matches!(a, Action::SetVolume(v) if *v > 0.50));
        assert!(has_vol, "Volume should have increased");

        // 4. Lifts finger
        let f4 = make_frame(250, vec![]);
        let acts4 = engine.process_frame(&f4, mods, 0.60, 0.50);
        assert!(!engine.is_sliding());
        assert_eq!(acts4, vec![Action::SlideEnd(SliderKind::Volume)]);
    }

    #[test]
    fn test_brightness_slide_gesture() {
        let mut engine = GestureEngine::new(Config::default());
        let mods = Modifiers::default();

        // Left edge touch (x = 0.03, y = 0.6)
        let f1 = make_frame(100, vec![Contact { id: 2, x: 0.03, y: 0.6, tip: true, confident: true }]);
        let _ = engine.process_frame(&f1, mods, 0.50, 0.50);

        // Move vertically
        let f2 = make_frame(140, vec![Contact { id: 2, x: 0.03, y: 0.55, tip: true, confident: true }]);
        let acts2 = engine.process_frame(&f2, mods, 0.50, 0.50);
        assert_eq!(acts2[0], Action::SlideStart(SliderKind::Brightness));
    }

    #[test]
    fn test_swap_sides() {
        let cfg = Config {
            swap_sides: true,
            ..Default::default()
        };
        let mut engine = GestureEngine::new(cfg);
        let mods = Modifiers::default();

        // With swap sides: Right edge (x = 0.95) should be Brightness!
        let f1 = make_frame(100, vec![Contact { id: 1, x: 0.95, y: 0.5, tip: true, confident: true }]);
        let _ = engine.process_frame(&f1, mods, 0.50, 0.50);
        let f2 = make_frame(140, vec![Contact { id: 1, x: 0.95, y: 0.45, tip: true, confident: true }]);
        let acts = engine.process_frame(&f2, mods, 0.50, 0.50);
        assert_eq!(acts[0], Action::SlideStart(SliderKind::Brightness));
    }

    #[test]
    fn test_bottom_quarter_only() {
        let cfg = Config {
            bottom_quarter_only: true,
            ..Default::default()
        };
        let mut engine = GestureEngine::new(cfg);
        let mods = Modifiers::default();

        // Top/middle of edge (y = 0.4) should NOT trigger
        let f1 = make_frame(100, vec![Contact { id: 1, x: 0.95, y: 0.4, tip: true, confident: true }]);
        let _ = engine.process_frame(&f1, mods, 0.50, 0.50);
        let f2 = make_frame(140, vec![Contact { id: 1, x: 0.95, y: 0.35, tip: true, confident: true }]);
        let acts = engine.process_frame(&f2, mods, 0.50, 0.50);
        assert!(acts.is_empty());
        assert!(!engine.is_sliding());

        // Bottom quarter (y = 0.85) should trigger
        let f3 = make_frame(200, vec![Contact { id: 2, x: 0.95, y: 0.85, tip: true, confident: true }]);
        let _ = engine.process_frame(&f3, mods, 0.50, 0.50);
        let f4 = make_frame(240, vec![Contact { id: 2, x: 0.95, y: 0.80, tip: true, confident: true }]);
        let acts2 = engine.process_frame(&f4, mods, 0.50, 0.50);
        assert_eq!(acts2[0], Action::SlideStart(SliderKind::Volume));
    }

    #[test]
    fn test_double_tap_mute_corner() {
        let mut engine = GestureEngine::new(Config::default());
        let mods = Modifiers::default();

        // Tap 1 down
        let f1 = make_frame(100, vec![Contact { id: 1, x: 0.95, y: 0.05, tip: true, confident: true }]);
        let _ = engine.process_frame(&f1, mods, 0.50, 0.50);
        // Tap 1 up at 150ms (duration 50ms)
        let f2 = make_frame(150, vec![]);
        let acts2 = engine.process_frame(&f2, mods, 0.50, 0.50);
        assert!(acts2.is_empty());

        // Tap 2 down at 250ms (interval 100ms)
        let f3 = make_frame(250, vec![Contact { id: 2, x: 0.95, y: 0.05, tip: true, confident: true }]);
        let _ = engine.process_frame(&f3, mods, 0.50, 0.50);
        // Tap 2 up at 300ms
        let f4 = make_frame(300, vec![]);
        let acts4 = engine.process_frame(&f4, mods, 0.50, 0.50);
        assert_eq!(acts4, vec![Action::ToggleMute]);
    }

    #[test]
    fn test_media_swipe() {
        let mut engine = GestureEngine::new(Config::default());
        let mods = Modifiers::default();

        // Touch lands in bottom strip (y = 0.95)
        let f1 = make_frame(100, vec![Contact { id: 1, x: 0.30, y: 0.95, tip: true, confident: true }]);
        let _ = engine.process_frame(&f1, mods, 0.50, 0.50);

        // Swipes right to x = 0.50 (> 0.15 delta) at t = 200ms
        let f2 = make_frame(200, vec![Contact { id: 1, x: 0.50, y: 0.95, tip: true, confident: true }]);
        let acts = engine.process_frame(&f2, mods, 0.50, 0.50);
        assert_eq!(acts, vec![Action::MediaNext]);
    }

    #[test]
    fn test_media_tap_to_play_pause_and_modifier() {
        let cfg = Config {
            tap_to_play_pause: true,
            require_modifier: ModifierRequirement::Alt,
            ..Default::default()
        };
        let mut engine = GestureEngine::new(cfg);

        // 1. Without required Alt modifier, touch in bottom strip is ignored
        let mods_none = Modifiers::default();
        let f1 = make_frame(100, vec![Contact { id: 1, x: 0.50, y: 0.95, tip: true, confident: true }]);
        let _ = engine.process_frame(&f1, mods_none, 0.50, 0.50);
        let f2 = make_frame(180, vec![]);
        let acts1 = engine.process_frame(&f2, mods_none, 0.50, 0.50);
        assert!(acts1.is_empty());

        // 2. With Alt modifier held, tapping bottom strip triggers MediaPlayPause
        let mods_alt = Modifiers { alt: true, ctrl: false, shift: false };
        let f3 = make_frame(300, vec![Contact { id: 2, x: 0.50, y: 0.95, tip: true, confident: true }]);
        let _ = engine.process_frame(&f3, mods_alt, 0.50, 0.50);
        // Finger lifted within TAP_MAX_DURATION_MS (80ms later)
        let f4 = make_frame(380, vec![]);
        let acts2 = engine.process_frame(&f4, mods_alt, 0.50, 0.50);
        assert_eq!(acts2, vec![Action::MediaPlayPause]);
    }

    #[test]
    fn test_palm_rejection_multi_finger() {
        let mut engine = GestureEngine::new(Config::default());
        let mods = Modifiers::default();

        // 3 fingers down
        let f1 = make_frame(100, vec![
            Contact { id: 1, x: 0.95, y: 0.5, tip: true, confident: true },
            Contact { id: 2, x: 0.50, y: 0.5, tip: true, confident: true },
            Contact { id: 3, x: 0.30, y: 0.5, tip: true, confident: true },
        ]);
        let acts = engine.process_frame(&f1, mods, 0.50, 0.50);
        assert!(acts.is_empty());
        assert!(!engine.is_sliding());
    }
}
