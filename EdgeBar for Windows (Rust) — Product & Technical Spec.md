# EdgeBar for Windows (Rust) — Product & Technical Spec

Nov 1, 2026 · @Umar

## 1. Overview

**Build a tiny, always-resident Windows tray app in Rust that turns the edges of a Precision Touchpad into volume and brightness sliders**, matching the behaviour of EdgeBar for Mac (as described on its public product page, v1.5, Oct 2026).

The Mac app reads multitouch data from the trackpad, treats the right edge as a volume slider and the left edge as a brightness slider, and shows the system on-screen display. Extras: quick mute, a media strip on the bottom edge, haptics, fine control, cursor freeze, bottom-quarter-only mode, swapped sides, and a menu-bar-only presence. The Windows version keeps the same interaction model and replaces each macOS API with its Windows equivalent.

**Goals**

- Same gestures and settings as EdgeBar 1.5, adapted to Windows modifiers (Alt for Option, Ctrl for Fn).
- Works on any laptop with a Windows Precision Touchpad (Windows 10 1903+ and Windows 11), x64 and ARM64.
- Single native binary, no admin rights, no Electron or .NET runtime, near-zero idle CPU.
- Smooth, low-latency sliding: under 16 ms from finger movement to volume or brightness change.

**Non-goals (v1)**

- Non-Precision touchpads (legacy Synaptics/Elan drivers without HID Precision reports).
- Touchscreens, mice, or external trackpads that do not report as Precision Touchpads.
- Cross-platform support (macOS/Linux). The Mac app already exists.
- Custom gesture editor or arbitrary actions per edge.

**Source of facts.** Everything about EdgeBar's behaviour comes from its marketing page; exact default values (edge width, sensitivity curve, haptic strengths) are not published, so this spec marks them as *proposed* and they should be tuned against the Mac app side by side. All Windows API choices below are my technical recommendations, not facts about the original app.

## 2. Feature parity matrix

**Almost every EdgeBar feature maps cleanly to Windows; haptics is the exception, because it depends on hardware support that Windows does not expose to apps in a general way.**

| EdgeBar (macOS) feature | Windows equivalent | Feasibility |
| --- | --- | --- |
| Right edge = volume slider | Raw HID touch contacts, Core Audio `IAudioEndpointVolume` | High |
| Left edge = brightness slider | WMI `WmiMonitorBrightnessMethods`, fallback DDC/CI via `dxva2` | High (internal panel); Medium (external monitors) |
| System-style on-screen display | Custom topmost, click-through layered window (Windows 11 style) | High |
| Quick Mute: double-tap top-right corner | Corner hit-test plus tap timing, `SetMute` | High |
| Media Strip: swipe bottom edge to skip, tap to play | `SendInput` media virtual keys, optional SMTC for per-app control | High |
| Haptic feedback while sliding | HID Haptics (usage page 0x0E) if the touchpad exposes it; otherwise none | Low to Medium, per device |
| Fine Control: hold Shift-Option for quarter steps | Hold Shift+Alt, scale slide gain by 0.25 | High |
| Cursor Freeze while sliding | Low-level mouse hook suppresses move events during a slide | Medium (see risks) |
| Bottom Quarter Only | Normalized y-range filter | High |
| Swap Sides | Config flag | High |
| Require Fn or Option modifier | Require Alt or Ctrl (Fn is not visible to software on most PCs) | High, with changed semantics |
| Menu bar only, no Dock icon | Notification-area (tray) icon, no taskbar button | High |
| Open at Login | `HKCU\\...\\Run` entry | High |
| Light/Dark appearance | Follow system theme, or force Light/Dark | High |
| Setup guide, auto-update, license keys | In-app first-run guide, signed self-updater, offline-verifiable key | High (needs a backend, see section 9) |

## 3. Architecture

**A touchpad frame passes through three stages: an input thread captures it, a pure Rust engine decides what it means, and action workers call Windows.** The UI never sits on that path.

&#91;embedded content: architecture · 3 threads, 4 action backends\]

Settings flow into the engine from `config.toml`; the UI thread only edits settings and draws the tray, flyout and OSD.

**Threads**

- **Input thread**: owns the hidden window, Raw Input registration, the optional low-level mouse hook, and its own message loop. It must never block, so it only parses and forwards frames.
- **Engine thread**: reads frames from a bounded channel, runs the state machine, and sends `Action`s to the action layer.
- **Action workers**: audio on a COM MTA thread, brightness on a latest-value-wins worker (DDC/CI can take 20 to 100 ms), media keys inline.
- **UI thread**: tray icon, flyout, Settings, setup guide and OSD windows.

**Workspace layout**

```text
edgebar/
  crates/
    edgebar-core   gesture engine, config schema, shared types (no OS dependencies)
    edgebar-win    Raw Input, HID parser, audio, brightness, media keys, mouse hook, OSD
    edgebar-ui     tray, flyout, Settings, setup guide
    edgebar-app    main(), wiring, single instance, license, updater
```

**Dependencies**

| Crate | Use |
| --- | --- |
| `windows` | Win32 and WinRT bindings (Raw Input, HID, Core Audio, WMI, DXVA2, Direct2D) |
| `tray-icon` | Notification-area icon and menu |
| `slint` | Flyout and Settings UI |
| `serde`, `toml` | Config file |
| `crossbeam-channel`, `smallvec` | Frame transport without allocation |
| `tracing`, `tracing-appender` | Logging |
| `thiserror`, `anyhow` | Error handling |
| `ed25519-dalek`, `sha2` | License and update signature checks |
| `ureq` (rustls) | Activation and update requests |

## 4. Touchpad input layer

**Capture finger positions from the Precision Touchpad's raw HID reports using the Win32 Raw Input API; Windows does not give apps per-finger coordinates any other way.** (macOS apps get this through the private MultitouchSupport framework; there is no private API needed on Windows.)

**4.1 Registration**

- Create a hidden top-level window (not message-only, to be safe with `RIDEV_INPUTSINK`) on a dedicated input thread with its own message loop.
- Call `RegisterRawInputDevices` with usage page `0x0D` (Digitizer), usage `0x05` (Touch Pad), flags `RIDEV_INPUTSINK | RIDEV_DEVNOTIFY`. `INPUTSINK` delivers reports while another app has focus, which is required for a background utility.
- Handle `WM_INPUT_DEVICE_CHANGE` to detect touchpads appearing or disappearing (docking, Bluetooth).

**4.2 Parsing a report**

1. On `WM_INPUT`, call `GetRawInputData(RID_INPUT)` into a reusable buffer and check `RIM_TYPEHID`.
2. For each device handle, cache its preparsed data once (`GetRawInputDeviceInfo` with `RIDI_PREPARSEDDATA`) and its capabilities via `HidP_GetCaps`, `HidP_GetValueCaps`, `HidP_GetButtonCaps` (from `hid.dll`).
3. Read per-report values with `HidP_GetUsageValue`: contact count (usage page `0x0D`, usage `0x54`), and per contact link collection: contact ID (`0x51`), X (generic desktop `0x30`), Y (`0x31`), plus tip switch (`0x42`) and confidence (`0x47`) when present.
4. Precision Touchpads report contacts in *hybrid mode*: a report carries up to a few contacts and a total contact count; keep a per-device table of active contact IDs and merge continuation reports until the count is satisfied.
5. Normalize using each axis's logical min/max from the value caps: `x = (raw - min) / (max - min)`, same for `y`, giving 0.0 to 1.0 with the origin at the **top-left** (flip Y if the device reports bottom-left). Also store physical size in 0.1 mm units if reported, used to make edge width physically consistent across pads.

**4.3 Output type**

```rust
pub struct Contact { pub id: u32, pub x: f32, pub y: f32, pub tip: bool, pub confident: bool }
pub struct TouchFrame { pub t: std::time::Instant, pub device: DeviceId, pub contacts: smallvec::SmallVec<[Contact; 5]> }
```

Frames are sent over a bounded `crossbeam` channel to the gesture engine; if the engine lags, drop the oldest frame (never block the window procedure).

**4.4 Edge cases to handle**

- Palm rejection: ignore contacts with `confident == false` and contacts that start inside the edge zone while 3 or more fingers are down.
- Multiple touchpads: track per device; use the device that produced the most recent frame.
- The touchpad is disabled in Settings or by a hotkey: no reports arrive; show a tray tooltip hint after 10 s of no frames following a setup-guide step.
- Windows still moves the cursor and fires its own gestures from the same contacts. Raw Input cannot consume them, which is why Cursor Freeze needs a separate mechanism (section 6.5).
- Devices whose HID descriptor lacks X/Y value caps are logged and skipped.

## 5. Gesture engine

**The engine is a pure, deterministic state machine: it takes `TouchFrame`s plus the current settings and modifier state, and emits `Action`s.** It has no Windows dependencies, so it can be unit-tested with recorded frames on any OS.

**5.1 Zones (normalized coordinates, origin top-left)**

| Zone | Region | Gesture | Action |
| --- | --- | --- | --- |
| Volume edge | `x >= 1 - edge_width` (right; left if Swap Sides) | Vertical slide | Set volume |
| Brightness edge | `x <= edge_width` (left; right if Swap Sides) | Vertical slide | Set brightness |
| Bottom strip | `y >= 1 - strip_height` | Horizontal swipe | Previous / next track |
| Bottom strip | `y >= 1 - strip_height` | Short tap (only if Tap to Play is on) | Play / pause |
| Mute corner | top-right `x >= 1 - corner`, `y <= corner` | Double tap within 350 ms | Toggle mute |

Proposed defaults: `edge_width` 0.08 (adjustable 0.04 to 0.20), `strip_height` 0.12, `corner` 0.15. With **Bottom Quarter Only** on, slider zones are additionally limited to `y >= 0.75`.

**5.2 Slider state machine**

1. **Idle**: a single new contact (tip down, confident) lands inside a slider zone and no other contact is within 25% of the pad width, so a palm or two-finger gesture does not trigger. Record `start_y`, `start_value` (read from the system), and a 60 ms *arming timer*.
2. **Armed**: if the contact leaves the zone sideways by more than 0.05, or a second finger lands, return to Idle. If it moves vertically by more than 0.015 (about 1.5% of pad height) inside the arming window, go to Sliding. If it lifts before moving, treat it as a possible tap and go to Idle.
3. **Sliding**: each frame emits `SetLevel(kind, value)` where `value = clamp(start_value + gain * (start_y - y) / zone_length, 0.0, 1.0)`. Sliding up raises the value. The finger may drift sideways out of the zone without ending the slide; it ends on lift or when more than one finger is down.
4. **Ending**: on lift, emit `SlideEnd` (starts the OSD hide timer) and return to Idle.

`zone_length` is 1.0 for the full edge, or 0.25 with Bottom Quarter Only, so a full sweep of the finger covers 0 to 100%. `gain` is `sensitivity` (proposed range 0.5 to 2.0, default 1.0), multiplied by 0.25 while Fine Control is held.

Smooth the output with a one-pole low-pass (`alpha` 0.35) and quantize volume to 1% steps and brightness to 1% steps before comparing with the last sent value; send only on change to avoid flooding the audio and WMI APIs.

**5.3 Modifier gating**

- **Require modifier** (off by default): slides only start while Alt (or Ctrl, chosen in Settings) is held. Read with `GetAsyncKeyState`, sampled when a contact lands.
- **Fine Control**: while Shift+Alt are both held, apply the 0.25 gain. Re-anchor `start_y` and `start_value` when the modifier state changes mid-slide so the value does not jump.

**5.4 Media strip**

A swipe is a contact that starts in the bottom strip and travels more than 0.15 horizontally within 400 ms: right sends Next, left sends Previous. A tap is a contact that lifts within 180 ms moving less than 0.02. Tap to Play or Pause defaults to **off**, as in EdgeBar 1.4, so ordinary clicks near the bottom edge do not pause music.

**5.5 Mute corner**

Two taps in the top-right corner, each under 180 ms with less than 0.02 travel, within 350 ms of each other, emit `ToggleMute`. A single tap does nothing.

## 6. System actions

**Each action sits behind a small trait so the gesture engine never touches Windows APIs directly, and each Windows backend can be swapped or mocked.**

```rust
pub trait VolumeControl { fn get(&self) -> Result<f32>; fn set(&self, v: f32) -> Result<()>; fn toggle_mute(&self) -> Result<bool>; }
pub trait BrightnessControl { fn get(&self) -> Result<f32>; fn set(&self, v: f32) -> Result<()>; }
pub trait MediaControl { fn next(&self); fn prev(&self); fn play_pause(&self); }
```

**6.1 Volume**

- Use the `windows` crate: `CoCreateInstance(MMDeviceEnumerator)`, `GetDefaultAudioEndpoint(eRender, eMultimedia)`, activate `IAudioEndpointVolume`.
- `SetMasterVolumeLevelScalar(v, null)` for volume and `SetMute` / `GetMute` for mute. Scalar 0.0 to 1.0 follows the same perceptual curve as the Windows volume slider.
- Register `IMMNotificationClient` for default-device changes and `IAudioEndpointVolumeCallback` for external volume changes, so the OSD and slider start value stay correct if the user changes volume elsewhere.
- Initialize COM (MTA) once on the action thread. Re-acquire the endpoint after a device change.

**6.2 Brightness (fallback chain)**

1. **Internal panel via WMI**: namespace `root\\WMI`, class `WmiMonitorBrightness` for the current level, `WmiMonitorBrightnessMethods::WmiSetBrightness(Timeout=0, Brightness=0..100)` to set. Keep the COM/WMI connection open for the process lifetime; creating it per call is too slow for sliding. Coalesce writes to at most one per 30 ms.
2. **Internal panel via IOCTL** if WMI is missing: open `\\\\.\\LCD`, `IOCTL_VIDEO_QUERY_SUPPORTED_BRIGHTNESS`, `IOCTL_VIDEO_SET_DISPLAY_BRIGHTNESS`.
3. **External monitors**: `GetPhysicalMonitorsFromHMONITOR` plus `GetMonitorBrightness` / `SetMonitorBrightness` (DDC/CI). These calls take 20 to 100 ms, so run them on a worker thread with latest-value-wins queueing, and apply to the monitor under the cursor, or to all monitors (Settings option).
4. If no backend works, the brightness edge is disabled and the tray menu shows "Brightness not available on this display".

The brightness slider should respect the user's display minimum: map 0.0 to the panel's lowest supported level, not to a black screen.

**6.3 Media keys**

Send `VK_MEDIA_NEXT_TRACK`, `VK_MEDIA_PREV_TRACK`, `VK_MEDIA_PLAY_PAUSE` with `SendInput` (key down and up, `KEYEVENTF_EXTENDEDKEY`). This controls whichever app Windows treats as the active media session. Optional v1.1: use `GlobalSystemMediaTransportControlsSessionManager` (WinRT) to target a specific session and show the track title in the OSD.

**6.4 On-screen display**

A custom OSD, since Windows only shows its own volume flyout for hardware keys. Specification:

- Layered, topmost, no-activate tool window (`WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT`), so it never steals focus or clicks.
- Rounded pill about 320 x 64 px at 100% DPI, centered horizontally near the bottom of the active monitor (Windows 11 style), with an icon (speaker, muted speaker, sun), a fill bar, and a percentage.
- Fade in 80 ms, hold 1200 ms after the last change, fade out 220 ms. Per-monitor DPI aware (v2). Rendered with Direct2D, drawn only while visible.
- Follows the system Light or Dark theme unless overridden.

**6.5 Cursor Freeze**

While a slide is in progress the pointer should not move. Raw Input cannot suppress it, so install a `WH_MOUSE_LL` hook (on the input thread, which already pumps messages) that returns 1 for `WM_MOUSEMOVE` events when the engine's state is *Sliding* and the event is not injected by another app. The hook callback must do nothing but read an atomic flag, because Windows removes hooks that take longer than the `LowLevelHooksTimeout` (default 300 ms). Fallback if the hook proves unreliable: `ClipCursor` to a 1 x 1 pixel rectangle at the current position for the duration of the slide, released on lift or after a 5 s safety timeout.

**6.6 Haptic feedback**

Windows has no general API for touchpad haptics. Plan: at startup, enumerate HID devices on usage page `0x0E` (Haptics) for the touchpad's parent device; if a waveform-capable haptic collection exists, send a short click waveform through `WriteFile` on the device handle or `Windows.Devices.Haptics.SimpleHapticsController` when available, with one tick per 2% of slider travel. If no haptic collection is found, hide the Haptics option and show "Not supported on this touchpad". Strength (Off, Light, Medium, Strong) maps to the waveform intensity. Treat this as an optional module that can ship after v1.

## 7. User interface

**The app has no main window: a notification-area icon opens a compact flyout that mirrors EdgeBar's menu, and a separate Settings page holds the set-once options.**

**7.1 Tray icon**

- `Shell_NotifyIconW` (via the `tray-icon` crate), monochrome glyph that adapts to light and dark taskbars, no taskbar button, no console window (`windows_subsystem = "windows"`).
- Left click opens the flyout; right click opens a native menu with Settings, Check for Updates, Manage License, Quit.
- Starts hidden-on-first-run only if Windows is configured to hide new tray icons; otherwise shown.

**7.2 Flyout (everyday controls)**

A borderless popup anchored above the tray icon, about 320 px wide, closing on focus loss or Esc. Contents, top to bottom:

1. A small touchpad diagram with two live bars: **brightness on the left edge, volume on the right edge**, updating as values change (this is EdgeBar 1.5's signature visual).
2. Toggles: Haptic Feedback (with a strength menu: Off, Light, Medium, Strong), Quick Mute, Media Strip.
3. Sliders: Sensitivity and Edge Width.
4. A "Settings..." button.

**7.3 Settings page (set-once options)**

A normal window, opened on demand and destroyed on close so it uses no memory afterward. Options: Swap Sides, Bottom Quarter Only, Require Modifier (None, Alt, Ctrl), Freeze Cursor, Fine Control, Tap to Play or Pause, Appearance (System, Light, Dark), Open at Login, License, About.

**7.4 UI toolkit**

Use a toolkit that renders on demand and costs nothing while hidden. Recommended: **Slint** with the software or Skia renderer for the flyout and Settings (declarative, small binary, good DPI support). Alternative: `egui`/`eframe` with repaint-on-demand. Avoid WebView and Electron-style approaches because they break the CPU and memory budget. The OSD (6.4) stays a hand-written Direct2D window because it must be click-through and never take focus.

**7.5 First-run setup guide**

A three-step guide shown once, matching EdgeBar 1.4: (1) confirm a Precision Touchpad was detected and show a live contact dot, (2) practice sliding the right edge to change volume, (3) practice sliding the left edge for brightness. Skippable, reopenable from Settings.

## 8. Configuration and lifecycle

**Settings live in one TOML file at `%APPDATA%\\EdgeBar\\config.toml`, loaded at start, written atomically on change (write to a temp file, then `MoveFileEx` replace), and hot-applied without a restart.**

| Setting | Type | Proposed default | Notes |
| --- | --- | --- | --- |
| `swap_sides` | bool | false | Brightness right, volume left |
| `bottom_quarter_only` | bool | false | Sliders only respond for `y >= 0.75` |
| `require_modifier` | none / alt / ctrl | none | Windows replacement for Fn or Option |
| `freeze_cursor` | bool | true | Section 6.5 |
| `fine_control` | bool | true | Shift+Alt gives 0.25 gain |
| `haptics` | off / light / medium / strong | medium (if supported) | Hidden when unsupported |
| `quick_mute` | bool | true | Double tap top-right |
| `media_strip` | bool | true | Bottom-edge swipe to skip |
| `tap_to_play_pause` | bool | false | Off by default, as in EdgeBar 1.4 |
| `sensitivity` | float 0.5 to 2.0 | 1.0 | Slider gain |
| `edge_width` | float 0.04 to 0.20 | 0.08 | Fraction of pad width |
| `appearance` | system / light / dark | system | Flyout, Settings, OSD |
| `open_at_login` | bool | true after setup guide | Mirrors the Run key |
| `brightness_target` | cursor\_monitor / all | cursor\_monitor | External monitors only |

**Lifecycle**

- **Single instance**: a named mutex (`Local\\EdgeBar.Singleton`); a second launch signals the first (named event) to open its flyout, then exits.
- **Open at login**: write or delete `HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run\\EdgeBar`. If distributed as MSIX, use the `StartupTask` API instead.
- **Power and session events**: handle `WM_POWERBROADCAST` (resume) and `WM_WTSSESSION_CHANGE` (lock and unlock) to re-register Raw Input and re-acquire audio and brightness endpoints, since both can go stale after sleep.
- **Logging**: `tracing` with a rolling file in `%LOCALAPPDATA%\\EdgeBar\\logs`, off by default except warnings; no touch coordinates are ever logged outside a debug build.
- **Privacy**: no network traffic except license activation and update checks (section 9); no keystroke or screen capture, and touch data never leaves the process.

## 9. Licensing, updates and distribution

**The Mac app is a $5 one-time purchase with a lifetime license, free 1.x updates, key-based activation, a "Manage activation" page, and self-installing updates. The Windows app should keep that model, but all backend pieces are out of scope for the app itself and need a decision.**

**9.1 License keys**

- Key format: a signed token (Ed25519 signature over `{product, key_id, email_hash, major_version}`), verified offline with a public key embedded in the binary, so the app works without internet after activation.
- Activation: a one-time HTTPS call to the license server that binds the key to a device ID (hash of the Windows `MachineGuid` and a salt) with a seat limit (proposed: 3 devices); "Manage activation" on the website releases seats.
- Unlicensed behaviour: proposed 14-day full-featured trial, then volume and brightness stay locked with a tray prompt. The Mac site does not document a trial, so this is a product decision.
- Store the activation record in `%APPDATA%\\EdgeBar\\license.json`, with the token signature protecting it from edits.

**9.2 Updates**

- Check a signed manifest (`https://<host>/windows/latest.json`) once a day and on demand; download the installer in the background; verify the Ed25519 signature on the manifest **and** the SHA-256 of the file before running it.
- Install silently at next quit or after the user confirms; never interrupt an active slide. Free updates within the same major version (1.x), as on Mac.
- Alternative: ship as MSIX through the Microsoft Store or direct App Installer, which gives Windows-managed updates and removes the need for a custom updater. This is the lowest-effort option.

**9.3 Build and signing**

- Targets: `x86_64-pc-windows-msvc` and `aarch64-pc-windows-msvc`, built in CI with `cargo build --release`, `lto = "fat"`, `panic = "abort"`, `strip = true`, static CRT.
- Sign the exe and installer with an EV or Azure Trusted Signing certificate; unsigned input utilities that install a mouse hook are routinely flagged by SmartScreen and antivirus.
- Installer: WiX (MSI) or MSIX, per-user install to `%LOCALAPPDATA%\\Programs\\EdgeBar`, no elevation.

**9.4 Payments**

The Mac site links to a Gumroad page while stating that Paddle is the merchant of record, so confirm which one actually handles checkout. Either can issue license keys by webhook to the license server above.

## 10. Non-functional requirements

**The app must feel invisible: under 0.5% of one core while idle, under 25 MB resident, and no perceptible lag while sliding.** These are proposed budgets, to be validated on a mid-range laptop.

| Area | Requirement |
| --- | --- |
| Idle CPU | Under 0.5% of one core; no timers running when no finger is down |
| Memory | Under 25 MB working set with UI closed; Settings window freed on close |
| Latency | Touch frame to volume change under 16 ms; to brightness (WMI) under 50 ms |
| Binary size | Under 8 MB, single exe |
| Startup | Tray icon visible within 300 ms of login; Raw Input registered within 500 ms |
| OS support | Windows 10 version 1903 or later, Windows 11; x64 and ARM64 |
| Privileges | Runs as a standard user; never requests elevation |
| DPI and displays | Per-monitor DPI v2; correct on mixed-DPI and multi-monitor setups |
| Reliability | No crash on device removal, sleep and resume, RDP sessions, or audio device switch; a watchdog restarts the input thread if frames stop while a finger is known to be down |
| Accessibility | Flyout is fully keyboard operable with screen-reader names; OSD respects the system animation setting |
| Localization | All strings in a resource table from day one (English first) |

## 11. Testing strategy

**Because the gesture engine is pure Rust with no OS calls, most of the product can be tested without a touchpad by replaying recorded frames.**

- **Unit tests**: zone hit-testing, slider math (including Fine Control re-anchoring and Bottom Quarter Only), tap and double-tap timing, palm rejection, Swap Sides. Use fake clocks, never `Instant::now()` inside the engine.
- **Golden replay tests**: a `--record` debug flag writes `TouchFrame` streams to a `.jsonl` file; tests replay them and assert the emitted `Action` sequence. Ship a corpus recorded on at least three touchpad vendors (Synaptics, Elan, Microsoft or Apple-class).
- **HID parser tests**: feed captured raw report bytes plus the matching report descriptor into the parser and check contact IDs and coordinates, including hybrid-mode multi-report frames.
- **Backend mocks**: implement `VolumeControl`, `BrightnessControl` and `MediaControl` fakes to assert call rates (for example at most one brightness write per 30 ms).
- **Manual device matrix**: 2 Intel laptops, 1 AMD, 1 Snapdragon (ARM64); Windows 10 and 11; one with an external DDC/CI monitor; one with Bluetooth audio output. Checklist per device: slide both edges, mute corner, bottom strip, sleep and resume, lock and unlock, unplug audio device mid-slide.
- **Performance tests**: measure idle CPU with `typeperf` and frame-to-action latency with an `Instant` probe at the channel boundaries; fail CI if the budgets in section 10 regress by more than 20%.

## 12. Milestones and acceptance criteria

**One engineer experienced in Rust and Win32 can reach a signed 1.0 in about 11 weeks; the first two are spent proving the two risky pieces, touchpad input and cursor freeze.** This is an estimate, not a commitment.

&#91;embedded content: roadmap · 4 phases, 4 gates\]

Each gate is a go or no-go check: if the prototype cannot read edge contacts on at least two touchpad vendors, stop and reassess before building the engine.

| Gate | Exit criteria |
| --- | --- |
| Prototype (weeks 1 to 2) | Raw Input delivers normalized contacts on three touchpad vendors; sliding the right edge changes volume; the cursor stays still during a slide on at least two of them |
| Core (weeks 3 to 5) | Engine passes all replay tests; volume change under 16 ms and WMI brightness under 50 ms from touch; OSD appears on the correct monitor at the correct DPI |
| Shell (weeks 6 to 8) | Tray, flyout and Settings change every option live; config persists across restarts; open at login works; media strip, mute corner and setup guide work |
| Ship (weeks 9 to 11) | Signed installer; license activation and auto-update tested end to end; manual device matrix passes; public beta feedback triaged; 1.0 released |

## 13. Risks, open questions and legal note

**The two largest technical risks are cursor freeze and the Precision Touchpad's own palm and edge handling; both should be prototyped in week 1, before any UI work.**

| Risk | Impact | Mitigation |
| --- | --- | --- |
| Cursor still moves while sliding (Raw Input cannot consume input) | Core feel of the product is worse | `WH_MOUSE_LL` suppression with an atomic flag; `ClipCursor` fallback; prototype first |
| Touchpad firmware rejects edge contacts as palms, so edge contacts never arrive or lack `confidence` | Slide never starts on some devices | Per-device calibration in the setup guide; widen edge width; document unsupported models |
| Antivirus or SmartScreen flags a background app with a global mouse hook | Install friction, support load | Code signing, no hook unless Freeze Cursor is on, publish a clear privacy statement |
| Haptics not exposed by most touchpad drivers | Feature missing on many laptops | Treat as optional, hide when unsupported, consider an audio tick option instead |
| WMI brightness absent on desktops and some external displays | Brightness edge unusable | Fallback chain in 6.2, clear tray message |
| Windows or driver updates change HID report layout | Parser breaks | Parse strictly from the report descriptor, never hard-code offsets; keep the replay corpus current |
| `RIDEV_INPUTSINK` reports not delivered during secure desktop (UAC, lock screen) | No gestures there | Accepted limitation; document it |
| Interaction with Windows' own three- and four-finger gestures | Accidental triggers | Require a single contact in the edge zone and ignore frames with 3 or more contacts |

**Open questions for the product owner**

- Trial period and device limit (section 9.1): decide or accept the proposals.
- Name and branding: see the legal note below.
- Is Fn-key gating dropped on Windows, or is a vendor-specific shortcut wanted?
- Should brightness apply to the monitor under the cursor, or all monitors by default?
- Distribution: direct download with a custom updater, or MSIX and the Microsoft Store?

**Legal note**

This spec describes behaviour observed from EdgeBar's public product page and does not use any of its code, icons, photos or copy. If this is meant to be a distinct product, give it its own name, icon and visual identity, and write original UI text; "EdgeBar" and the Mac app's art belong to its author. If it is meant to be the official Windows port, agree the plan with the author first.
