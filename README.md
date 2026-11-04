# Clickr

A high-performance, ultra-lightweight Windows autoclicker written in Rust using pure native Win32 controls (`user32` / `comctl32` / `gdi32`). Zero web wrappers or GUI framework overhead.

## Performance
- **~335 KB Binary:** Standalone single-file executable with zero external dependencies.
- **~1.8 MB RAM:** Minimal memory footprint (<5 MB working set).
- **0% Idle CPU & 0% GPU:** Pure GDI rendering and kernel-wait message loop when idle; <1% CPU when active.
- **High-Precision Timing:** Monotonic drift-compensated loop (`timeBeginPeriod(1)`) for exact click rates up to 100 CPS with lag-spike protection and instant (<2 ms) stop response.

## Features
- **Trigger Modes:**
  - **Toggle:** Press hotkey once to start, again to stop.
  - **Hold Hotkey:** Clicks only while the global hotkey is held down.
  - **Hold Mouse:** Clicks while holding your physical mouse button (uses `WH_MOUSE_LL` to ignore synthetic clicks).
- **Flexible Timing:** Set by **Click Rate** (CPS, CPM, CPH — max 100 CPS) or exact **Click Interval** (`hr`, `min`, `sec`, `ms`).
- **Mouse & Keyboard Support:**
  - **Mouse:** Left, Right, or Middle button (`Single`, `Double`, or `Hold`).
  - **Keyboard:** Any key (`Press` or `Hold`), with fail-safe release guards so inputs never get stuck.
- **Cursor Location:** Click at the **Current** cursor position or lock to **Specific (X, Y)** multi-monitor coordinates with an interactive screen picker.
- **Repeat Limit:** Run indefinitely (`0`) or stop automatically after `N` clicks/presses.
- **Quality of Life:** Custom global hotkeys, native Fluent Dark Mode, hover tooltips, and automatic `.ini` persistence for settings and window position.

## Build & Run
Requires **Rust 1.70+**.

```powershell
# Build optimized release binary (outputs to target\release\clickr.exe)
cargo build --release

# Run unit tests
cargo test
```

## Project Structure
- [src/main.rs](src/main.rs) — Window creation, dark title bar, and Win32 message loop.
- [src/ui.rs](src/ui.rs) — Control layout, dark theme styling, and custom GDI button rendering.
- [src/clicker.rs](src/clicker.rs) — SendInput simulation, drift-compensated timer, and worker thread.
- [src/hotkeys.rs](src/hotkeys.rs) — Global hotkeys (RegisterHotKey) and low-level mouse/keyboard hooks.
- [src/settings.rs](src/settings.rs) — INI state and window position persistence.
- [build.rs](build.rs) & [app.manifest](app.manifest) — Icon generation, resource embedding, and PerMonitorV2 DPI config.

## License
[MIT License](LICENSE)
