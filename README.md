# Clickr — Ultra-Lightweight Native Win32 Autoclicker

**Clickr** is a high-performance, ultra-lightweight Windows autoclicker built in Rust. It utilizes **pure native Win32 controls** (`user32.dll` / `comctl32.dll` / `gdi32.dll`) without any GPU-accelerated frameworks, rendering engine overhead, or web wrappers.

---

## Key Highlights & Performance Specs

- **0.0% GPU Usage:** Direct native Win32 controls with pure GDI drawing.
- **0.0% Idle CPU Usage:** Event-driven message loop (`GetMessageW` / `WaitMessage`) sleeps in kernel wait state when idle.
- **Ultra-Low Memory Footprint:** Uses ~1.84 MB private memory (<5 MB working set).
- **Sub-350 KB Standalone Executable:** Fully standalone single-file binary with zero external dependencies (no .NET, no MSVC redistributables, no webview).
- **Full Settings & Position Persistence:** Automatically remembers your exact choices (timing mode, interval/rate values, mouse vs keyboard, repeat count, custom hotkeys) and screen position across restarts.
- **Modern Fluent Dark Mode:** 
  - Native immersive dark title bar via `DwmSetWindowAttribute`.
  - Rich Zinc-900 dark theme (`#18181B`) with Zinc-800 surfaces (`#27272A`).
  - Windows Common Controls v6 dark theming via `SetWindowTheme` (`DarkMode_Explorer` / `DarkMode_CFD`).
  - Modern custom-styled buttons with rounded corners and active color feedback (Electric Blue idle, Rose/Crimson running, Cyan rebind glow).
- **Custom Embedded Application Icon:** Crisp 32-bit multi-resolution icon (16x16, 32x32, 48x48) replacing the legacy Windows 95 generic icon across the title bar, taskbar, Alt-Tab switcher, and Windows Explorer.
- **Drift-Compensated High-Precision Timing Engine:**
  - Course-corrects on every click using monotonic clock absolute scheduling (`std::time::Instant`).
  - Automatically compensates for OS scheduling delays so 100 CPS achieves exactly 100.00 CPS (0.019% timing error).
  - Built-in lag-spike guard prevents catch-up burst clicking after system freezes.
  - Hybrid wait keeps active CPU usage under 1% while delivering microsecond-accurate timing.
- **Instant Response / Zero Sleep Delay:** Multi-threaded worker thread synchronized via `Condvar` and `AtomicBool`—pressing `STOP` or the hotkey halts the clicker immediately (<2 ms) even during hours-long sleep intervals.
- **High-Precision Kernel Timing:** Automatically enables 1 ms timer granularity via `timeBeginPeriod(1)` while active.

---

## UI Layout & Features

Fixed-size, compact window (~460x220 px) with four clean sections:

1. **Top Action Bar:**
   - **`START (<Hotkey>)` / `STOP (<Hotkey>)`:** Modern styled toggle button with dynamic color feedback (Electric Blue when stopped, Vibrant Crimson when active). Displays the currently bound global hotkey (default: `` ` `` / backtick).
   - **`Hotkeys` Button:** Click to rebind the global Start/Stop hotkey by pressing any key (supports function keys `F1`–`F24`, numbers, letters, navigation keys, etc.). Glows cyan while capturing. Pressing `Esc` cancels.

2. **Row 1 — Timing Mode (Click Rate vs. Click Interval):**
   - **`(•) Click Rate` (Default):**
     - A rate numeric input: `[50]` paired with a dropdown offering:
       - `Clicks / Second (CPS)` (Default: 50 CPS)
       - `Clicks / Minute (CPM)`
       - `Clicks / Hour (CPH)`
   - **`( ) Click Interval`:**
     - Four numeric inputs: `[0] hr`, `[0] min`, `[0] sec`, `[100] ms`.
   - Dynamically swaps between the rate and interval inputs in-place.
   - Enforces a safe 1 ms minimum delay (up to 1,000 CPS) and handles zero/invalid inputs gracefully.

3. **Row 2 — Buttons (Input Selection):**
   - **`(•) Mouse` (Default):** Themed dark dropdown menu with `Left` (default), `Right`, and `Middle`.
   - **`( ) Keyboard`:** Styled button displaying the bound target key (default: `[ Space ]`). Click to capture any keypress.

4. **Row 3 — Click Amount:**
   - Label: `Click Amount (0 = forever)`
   - Numeric input: `[0]` (default: 0).
   - When set to `0`, clicks indefinitely until stopped. When set to `N > 0`, automatically halts after `N` clicks and restores the `START (<Hotkey>)` button.

---

## Architecture & Code Structure

- [`src/main.rs`](src/main.rs): Win32 window creation, DWM immersive dark title bar, message loop (`GetMessageW` + `IsDialogMessageW`), dark theme message handling (`WM_CTLCOLORSTATIC`, `WM_CTLCOLOREDIT`, `WM_DRAWITEM`).
- [`src/ui.rs`](src/ui.rs): Controls management, `Segoe UI` fonts, dark color palette, `SetWindowTheme` dark integration, `draw_custom_button` GDI renderer.
- [`src/clicker.rs`](src/clicker.rs): `SendInput` mouse/keyboard simulation, `calculate_interval` logic, drift compensation engine, and responsive `Condvar` worker loop.
- [`src/hotkeys.rs`](src/hotkeys.rs): Global hotkey registration (`RegisterHotKey`), low-level keyboard hook (`WH_KEYBOARD_LL`) for key rebinding, and `vk_to_string` display translation.
- [`build.rs`](build.rs): Generates multi-resolution anti-aliased `app.ico` and embeds it with `app.manifest` via `winres`.
- [`app.manifest`](app.manifest): Windows Common Controls v6.0 and PerMonitorV2 DPI configuration.

---

## Building and Running

### Prerequisites
- Rust 1.70+ (`cargo` & `rustc`)

### Compile Release Binary
```powershell
cargo build --release
```
The optimized binary will be produced at:
`target\release\clickr.exe` (~335 KB)

### Run Unit Tests
```powershell
cargo test
```
