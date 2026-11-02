# Sidewise (EdgeBar for Windows in Rust)

**Sidewise** is a lightweight, always-resident Windows utility in Rust that transforms the edges of any Windows Precision Touchpad into smooth volume and brightness sliders, with quick mute, media strip control, cursor freeze, fine control, custom on-screen display (OSD), and tray management.

---

## ⚡ Highlights & Features

- **Right Edge = Volume Slider**: Slide up/down along the right edge to adjust system audio smoothly.
- **Left Edge = Brightness Slider**: Slide up/down along the left edge to adjust display brightness.
- **System-Style On-Screen Display (OSD)**: Custom layered, click-through pill (`WS_EX_LAYERED | WS_EX_TRANSPARENT`) positioned in Windows 11 style with smooth fade-in (80 ms), hold (1200 ms), and fade-out (220 ms).
- **Quick Mute**: Double-tap the top-right corner to instantly toggle mute.
- **Media Strip**: Swipe horizontally along the bottom edge of the touchpad to skip tracks (right for Next, left for Previous); optional short tap to play/pause.
- **Cursor Freeze**: Low-level mouse hook (`WH_MOUSE_LL`) and pointer clipping suppress cursor movement during active slides.
- **Fine Control**: Hold `Shift + Alt` mid-slide to apply a `0.25x` gain for micro adjustments without value jumps.
- **Swap Sides**: Invert edges (left for volume, right for brightness).
- **Bottom Quarter Only Mode**: Restricts edge sliders to `y >= 75%` to keep normal trackpad gestures completely untouched on top.
- **Activation Modifiers**: Optional requirement to hold `Alt` or `Ctrl` to trigger edge sliders.
- **Fluent Tray Icon & Live Flyout**: Anchored above the system tray with a live touchpad visualization, quick toggles, sensitivity and edge-width sliders.
- **First-Run Setup Guide**: Interactive 3-step practice window with a real-time touch visualizer.
- **Zero Runtime Overhead**: Single self-contained native binary under 3 MB, no admin privileges required, no Electron or .NET runtime, < 0.5% idle CPU.

---

## 🏗️ Architecture

```
sidewise/
  ├── Cargo.toml
  ├── crates/
  │     ├── sidewise-core/       Pure Rust gesture engine, config schema, shared types (no OS dependencies)
  │     ├── sidewise-win/        Win32 Raw Input, HID parser, Core Audio, Brightness, Media, Mouse Hook, OSD
  │     ├── sidewise-ui/         System tray icon, live flyout popup, preferences window, setup guide
  │     └── sidewise-app/        main(), lifecycle, single-instance mutex, worker threads
```

### Threading Model

1. **Input Thread**: Owns the hidden window, Raw Input registration for Precision Touchpads (`0x0D / 0x05`), and low-level mouse hook message pump.
2. **Engine Thread**: Receives touch frames over a bounded channel, executes the pure state machine, manages arming/filtering/smoothing, and dispatches actions.
3. **Action Workers**: Core Audio on COM MTA, throttled brightness worker with latest-value-wins queueing, and media keys via `SendInput`.
4. **UI Thread**: Owns the tray icon, layered acrylic OSD, live flyout, settings window, and interactive onboarding guide.

---

## 🛠️ Building & Running

### Requirements
- Windows 10 (1903+) or Windows 11
- Laptop with a Windows Precision Touchpad (or external Precision Trackpad)
- Rust 1.80+ (MSVC toolchain recommended)

### Build Commands
```powershell
# Build release binary (single .exe under 3 MB)
cargo build --release

# Run tests
cargo test

# Launch Sidewise
.\target\release\sidewise.exe

# Launch directly into the 3-step setup guide
.\target\release\sidewise.exe --setup
```

---

## ⚙️ Configuration

Settings are stored in `%APPDATA%\Sidewise\config.toml` and applied hot without restart:

```toml
swap_sides = false
bottom_quarter_only = false
require_modifier = "none" # "none" | "alt" | "ctrl"
freeze_cursor = true
fine_control = true
haptics = "medium"        # "off" | "light" | "medium" | "strong"
quick_mute = true
media_strip = true
tap_to_play_pause = false
sensitivity = 1.0         # 0.5 to 2.0
edge_width = 0.08         # 0.04 to 0.20 (percentage of pad width)
appearance = "system"     # "system" | "light" | "dark"
open_at_login = true
brightness_target = "cursor_monitor"
```
