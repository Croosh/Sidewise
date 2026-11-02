use sidewise_core::descriptor::parse_report_descriptor;
use sidewise_core::gesture::GestureEngine;
use sidewise_core::profile::{DeviceIdentity, ProfileDatabase};
use sidewise_core::types::{Action, Contact, DeviceId, Modifiers, SliderKind, TouchFrame};
use sidewise_core::Config;
use smallvec::smallvec;

#[test]
fn test_bundled_profiles_load() {
    let db = ProfileDatabase::new();
    assert!(!db.all_profiles().is_empty());

    // Check Surface profile is in bundled list
    let surface_ident = DeviceIdentity {
        vid: 0x045E,
        pid: 0x096F,
        descriptor_hash: "dummyhash123".into(),
        name: "Surface Precision Touchpad".into(),
    };
    let prof = db.lookup(&surface_ident);
    assert_eq!(prof.id, "microsoft-surface-pts");
    assert!(prof.defaults.confidence);
    assert_eq!(prof.fields.x.usage, 0x30);
    assert_eq!(prof.fields.y.usage, 0x31);
}

#[test]
fn test_lookup_order() {
    let mut db = ProfileDatabase::new();

    // Add a custom profile with exact descriptor hash
    let toml_custom = r#"
[[profile]]
id = "exact-hash-match"
source = "probe"

[profile.match]
vid = 0x1234
pid = 0x5678
descriptor_hash = "abcdef123456"
name = "Exact Match Test"

[profile.fields.x]
type = "value"
usage_page = 0x01
usage = 0x30

[profile.fields.y]
type = "value"
usage_page = 0x01
usage = 0x31

[[profile]]
id = "vid-pid-match"
source = "bundled"

[profile.match]
vid = 0x1234
pid = 0x5678
name = "VID PID Match Test"

[profile.fields.x]
type = "value"
usage_page = 0x01
usage = 0x30

[profile.fields.y]
type = "value"
usage_page = 0x01
usage = 0x31
"#;
    db.load_from_toml(toml_custom, "test");

    // 1. Should match exact hash
    let ident_exact = DeviceIdentity {
        vid: 0x1234,
        pid: 0x5678,
        descriptor_hash: "abcdef123456".into(),
        name: "Test Device".into(),
    };
    let prof1 = db.lookup(&ident_exact);
    assert_eq!(prof1.id, "exact-hash-match");

    // 2. Should match vid+pid if hash differs
    let ident_other_hash = DeviceIdentity {
        vid: 0x1234,
        pid: 0x5678,
        descriptor_hash: "different_hash".into(),
        name: "Test Device".into(),
    };
    let prof2 = db.lookup(&ident_other_hash);
    assert_eq!(prof2.id, "vid-pid-match");

    // 3. Unknown device should fallback to generic
    let ident_unknown = DeviceIdentity {
        vid: 0x9999,
        pid: 0x8888,
        descriptor_hash: "hash".into(),
        name: "Unknown Device".into(),
    };
    let prof3 = db.lookup(&ident_unknown);
    assert!(prof3.id.starts_with("generic-"));
    assert_eq!(prof3.source, "generic");
}

#[test]
fn test_malformed_profile_skipped_and_valid_still_loads() {
    let mut db = ProfileDatabase::new();
    let initial_count = db.all_profiles().len();

    let toml_with_bad_entry = r#"
# Malformed: missing x and y fields
[[profile]]
id = "bad-broken-profile"
[profile.match]
vid = 0x9999
pid = 0x0001
[profile.fields.x]
type = "value"
usage_page = 0
usage = 0
[profile.fields.y]
type = "value"
usage_page = 0
usage = 0

# Valid entry
[[profile]]
id = "good-profile"
[profile.match]
vid = 0x9999
pid = 0x0002
[profile.fields.x]
type = "value"
usage_page = 0x01
usage = 0x30
[profile.fields.y]
type = "value"
usage_page = 0x01
usage = 0x31
"#;

    db.load_from_toml(toml_with_bad_entry, "test_mixed");
    // The good profile should be added, the bad profile skipped
    assert_eq!(db.all_profiles().len(), initial_count + 1);

    let ident = DeviceIdentity {
        vid: 0x9999,
        pid: 0x0002,
        descriptor_hash: "hash".into(),
        name: "Good".into(),
    };
    let looked_up = db.lookup(&ident);
    assert_eq!(looked_up.id, "good-profile");
}

#[test]
fn test_surface_missing_confidence_fixture_replay_starts_slide() {
    // 1. Synthesize a Surface-like HID report descriptor without Confidence (0x47)
    // Usage Page (Digitizer 0x0D), Usage (Touch Pad 0x05)
    // Usage (Tip Switch 0x42, Button)
    // Generic Desktop X, Y
    let raw_descriptor = &[
        0x05, 0x0D, // USAGE_PAGE (Digitizer)
        0x09, 0x05, // USAGE (Touch Pad)
        0xA1, 0x01, // COLLECTION (Application)
        0x05, 0x0D, //   USAGE_PAGE (Digitizer)
        0x09, 0x42, //   USAGE (Tip Switch)
        0x15, 0x00, //   LOGICAL_MINIMUM (0)
        0x25, 0x01, //   LOGICAL_MAXIMUM (1)
        0x75, 0x01, //   REPORT_SIZE (1)
        0x95, 0x01, //   REPORT_COUNT (1)
        0x81, 0x02, //   INPUT (Data,Var,Abs)
        0x05, 0x01, //   USAGE_PAGE (Generic Desktop)
        0x09, 0x30, //   USAGE (X)
        0x09, 0x31, //   USAGE (Y)
        0x16, 0x00, 0x00, // LOGICAL_MINIMUM (0)
        0x26, 0x00, 0x10, // LOGICAL_MAXIMUM (4096)
        0x75, 0x10, //   REPORT_SIZE (16)
        0x95, 0x02, //   REPORT_COUNT (2)
        0x81, 0x02, //   INPUT (Data,Var,Abs)
        0xC0,       // END_COLLECTION
    ];

    let parsed_desc = parse_report_descriptor(raw_descriptor);
    // Verify Confidence (0x47) is absent from descriptor
    assert!(!parsed_desc.has_confidence);

    // Look up device profile
    let db = ProfileDatabase::new();
    let surface_ident = DeviceIdentity {
        vid: 0x045E,
        pid: 0x096F,
        descriptor_hash: "surface_pts_hash".into(),
        name: "Surface PTS".into(),
    };
    let profile = db.lookup(&surface_ident);
    assert_eq!(profile.id, "microsoft-surface-pts");

    // Simulated frame parsing:
    // With Confidence absent, profile.defaults.confidence is applied (true)
    let default_confident = profile.defaults.confidence;
    assert!(default_confident);

    // Feed sequence of frames into GestureEngine to assert slide begins
    let mut engine = GestureEngine::new(Config::default());
    let dev_id = DeviceId(101);

    // Frame 1: Touch down on right edge (x = 0.96, y = 0.50, tip = true, confident = default_confident)
    let frame1 = TouchFrame {
        t_ms: 100,
        device: dev_id,
        contacts: smallvec![Contact {
            id: 1,
            x: 0.96,
            y: 0.50,
            tip: true,
            confident: default_confident,
        }],
    };
    let _actions1 = engine.process_frame(&frame1, Modifiers::default(), 0.5, 0.5);
    // Slide should be armed, not yet sliding
    assert!(!engine.is_sliding());

    // Frame 2: Slide up (x = 0.96, y = 0.44) -> transitions from Armed to Sliding
    let frame2 = TouchFrame {
        t_ms: 130,
        device: dev_id,
        contacts: smallvec![Contact {
            id: 1,
            x: 0.96,
            y: 0.44,
            tip: true,
            confident: default_confident,
        }],
    };
    let actions2 = engine.process_frame(&frame2, Modifiers::default(), 0.5, 0.5);

    // Assert that slide starts
    assert!(engine.is_sliding());
    assert_eq!(actions2[0], Action::SlideStart(SliderKind::Volume));

    // Frame 3: Continue sliding up to y = 0.20 -> Adjusts Volume
    let frame3 = TouchFrame {
        t_ms: 160,
        device: dev_id,
        contacts: smallvec![Contact {
            id: 1,
            x: 0.96,
            y: 0.20,
            tip: true,
            confident: default_confident,
        }],
    };
    let actions3 = engine.process_frame(&frame3, Modifiers::default(), 0.5, 0.5);
    assert!(actions3.iter().any(|a| matches!(a, Action::SetVolume(v) if *v > 0.5)));
}

#[test]
fn test_stuck_edge_contact_health_check() {
    let mut engine = GestureEngine::new(Config::default());
    let dev_id = DeviceId(202);

    // Contact down in left edge zone but doesn't move
    let frame1 = TouchFrame {
        t_ms: 1000,
        device: dev_id,
        contacts: smallvec![Contact {
            id: 2,
            x: 0.04,
            y: 0.50,
            tip: true,
            confident: true,
        }],
    };
    let actions1 = engine.process_frame(&frame1, Modifiers::default(), 0.5, 0.5);
    assert!(actions1.is_empty());

    // 700ms later: still in place without sliding
    let frame2 = TouchFrame {
        t_ms: 1700,
        device: dev_id,
        contacts: smallvec![Contact {
            id: 2,
            x: 0.04,
            y: 0.50,
            tip: true,
            confident: true,
        }],
    };
    let actions2 = engine.process_frame(&frame2, Modifiers::default(), 0.5, 0.5);
    assert!(actions2.iter().any(|a| matches!(a, Action::TouchpadDiagnostic(_))));
}
