# Sidewise

Sidewise is a small, quiet Windows utility written in Rust. It lets you slide along the outer edges of your laptop touchpad to control volume and screen brightness, without having to reach for function keys or open quick settings.

It also includes horizontal track skipping along the bottom edge, a custom on-screen display, cursor freeze while sliding, and a system tray flyout for quick adjustments.

---

## What It Does

- **Right edge volume**: Slide a finger up or down along the right edge of your trackpad to change audio volume.
- **Left edge brightness**: Slide along the left edge to adjust display brightness.
- **Bottom edge media strip**: Swipe left or right along the bottom border to skip tracks backward or forward. You can also turn on tap-to-play/pause if you want a quick play/pause target.
- **Double-tap mute**: Quickly double-tap the top right corner to mute or unmute audio.
- **Cursor freeze**: While you are actively sliding an edge, your mouse cursor stays locked in place so you do not accidentally click off-screen.
- **Fine control**: Hold `Shift + Alt` while sliding to adjust at 0.25x speed for precision increments.
- **Clean OSD**: A minimal volume and brightness pill shows your current level and fades away automatically.
- **Tray flyout & settings**: Click the system tray icon to view live touchpad levels, toggle options on the fly, adjust sensitivity, or invert sides.
- **Lightweight**: Single native executable with low CPU and memory footprints. No Electron, no .NET requirements, and no background bloat.

---

## Requirements

- Windows 10 (version 1903 or later) or Windows 11
- A laptop with a Windows Precision Touchpad (or a compatible external precision trackpad)

---

## Installation

### Installer
Download `Sidewise-v1.0.0-Setup.exe` from the Releases page and run it. The installer adds shortcuts and gives you the option to start Sidewise when Windows boots.

### Portable
Download `Sidewise-v1.0.0-Portable.zip`, unzip it anywhere, and launch `sidewise.exe`. No installation required.

---

## How It Works

Sidewise listens to raw multi-touch input directly through Windows Precision Touchpad APIs (`WM_INPUT` with HID usage `0x0D / 0x05`). It watches incoming contact coordinates and runs them through a state machine:

1. **Edge detection**: Contacts starting within the configured margin (default 8% of pad width) qualify for gesture tracking.
2. **Filtering & rejection**: Multi-finger gestures, palms, and swipes that stray too far into the center of the trackpad are ignored, handing normal cursor movement back to Windows.
3. **Smooth output**: Active edge slides stream directly to Windows Core Audio (`IAudioEndpointVolume`) or the display brightness worker (WmiMonitorBrightnessMethods / DXVA2 monitor APIs).

---

## Configuration

Preferences are stored in a simple TOML file located at `%APPDATA%\Sidewise\config.toml`:

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
edge_width = 0.08         # 0.04 to 0.20
appearance = "system"     # "system" | "light" | "dark"
open_at_login = true
brightness_target = "cursor_monitor"
```

Changes made through the settings window apply immediately without needing to restart the app.

---

## Building From Source

If you prefer to compile Sidewise yourself:

1. Make sure you have Rust installed (1.80+ with MSVC toolchain).
2. Clone the repository:
   ```powershell
   git clone https://github.com/Croosh/Sidewise.git
   cd Sidewise
   ```
3. Run tests:
   ```powershell
   cargo test
   ```
4. Build an optimized release binary:
   ```powershell
   cargo build --release
   ```
   The finished binary will be at `target/release/sidewise.exe`.

---

## License

This project is licensed under the MIT License.
