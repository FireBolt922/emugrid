# Plan: UI redesign — build the full design

**Status:** Proposed · **Design:** claude.ai/artifact/7Hq9ra1Y5HsPtoLWg2G8tw ("emugrid UI Redesign", 5 boards)

## Context

The redesign canvas specifies five boards: **1 · Grid (macOS)**, **2 · Focused device**,
**3 · Device list (Ubuntu)**, **4 · Empty & setup states** and **5 · Visual language & egui
mapping**. Commit `aefef73` built the foundation (palette, painted icons, button kinds,
segmented control, pills, bezel, split button, control rail, list table, status bar). The
first version of this plan only covered the most visible gaps. A board-by-board comparison
found much more: data the design shows but the app never collects (Android version/API,
fps, boot state, SDK search results), keyboard shortcuts, zoom modes, the install toast,
the Wipe confirmation, window chrome and fonts. This plan covers the whole design.

**Hard constraint:** binary ≤ 8 MB (today's release build: **6.9 MB**). Add no crates.

### Decisions (confirmed with the user)
- Scope: everything on the canvas, including the sidebar.
- SDK folder: a **text field** + Re-check. No native file-dialog crate.
- Window chrome: **macOS unified toolbar** (full-size content view, traffic lights inset).
  **Linux/Windows keep the native title bar**; the Ubuntu board's "Main menu" (hamburger)
  goes into our toolbar. No custom drag/resize/window buttons.
- Fonts: **swap egui's default fonts for Latin subsets** of Inter (Regular, SemiBold) and
  JetBrains Mono (Regular). It has to stay roughly size-neutral; measure it.

### My calls (correct me if any are wrong)
- **Install APK button** opens a small popover with a path text field, "Install on N
  devices", and the hint "or drop an .apk anywhere". This follows the no-dialog decision.
- **Shortcuts in the focused view need ⌘ (macOS) / Ctrl (others).** Keys go to the device
  there, so the filmstrip's bare "1–5" become ⌘1–⌘9. The grid view keeps bare 1–9.
- **"Wipe data and start…"** opens a confirmation dialog first (the design's ellipsis +
  "destructive").
- **Settings persist** in a tiny hand-written `key=value` file (SDK path, tile size,
  sidebar on/off). No serde. It also sets up the roadmap's "remember window layout".
- **Theme follows the system** (the style guide says so). The board's dark/light tweak is
  only for previewing the canvas.
- **Filmstrip thumbnails** show each device's last frame and do not stream, so a focused
  device costs the same as today.
- **Token deviations:** where `theme.rs` differs from the style guide (e.g. `raised`
  `#2A313C` vs `#232933`, `accent_soft`, light `live`), match the design unless
  `text_meets_contrast` fails. The test decides, and each kept deviation gets a comment.
- The stray `https://github.com/…/oxishell` text on the Empty board is a canvas accident
  and is ignored.

---

## Phase 0 — Foundations (data + infrastructure)

All later phases depend on these. Each item has unit tests.

1. **Split `src/app.rs`** (1735 lines; it would double) into `src/ui/{grid,focused,list,setup,sidebar,toasts}.rs`.
   `EmuGrid` state, `Action`, `apply` and `ui()` stay in `app.rs`. Move code first and change
   nothing (one step), then build. Add the new files to the `ui_symbols_have_glyphs` source list.
2. **Device info** (`src/devices.rs`, new `src/avd.rs`):
   - Running Android: extend the existing single `props()` adb call to
     `getprop ro.product.model; ro.kernel.qemu; ro.build.version.release; ro.build.version.sdk; sys.boot_completed`.
     It is still one adb call per device per scan.
   - Stopped AVDs: read the AVD home (`$ANDROID_AVD_HOME` → `$ANDROID_USER_HOME/avd` → `~/.android/avd`),
     `<name>.ini` `path=` → `config.ini`: `avd.ini.displayname`, `image.sysdir.1`
     (`system-images/android-35/…`), `tag.id` (`android-wear` → Wear OS).
   - New `Device` fields: `display_name` ("Pixel 8"; the AVD id stays in `avd`), `api: Option<u32>`,
     `os: Option<String>` ("Android 15", "Wear OS 5", "iOS 18.2"). Add a small API → version table.
   - **Booting:** new `State::Booting { serial }` when adb says `device` but `sys.boot_completed != 1`.
     `EmuGrid.starts: HashMap<avd, (Instant, Boot)>` records Starts we issued, so a booting tile can
     show "cold boot · 0:41". An AVD we started that adb doesn't list yet also shows as booting.
3. **Fps meter**: sample `capture::Control.frames` (already counted) about once a second per serial.
   That feeds the pill labels `LIVE 30 fps` / `2 fps` / `BOOTING` / `STOPPED` / `VIEW ONLY`,
   the sidebar meta and the inspector stat. It replaces today's `LIVE` / `PNG` / `STARTING`
   in `status_badge`.
4. **SDK diagnosis** (`src/sdk.rs`): `Sdk` gains `root: Option<PathBuf>` and an optional override
   root (from settings, checked first). `diagnose()` returns each candidate with its result
   (`$ANDROID_HOME` not set, `~/Android/Sdk` no such folder, `adb on $PATH` not found, …).
   It reuses `sdk_roots()` / `find_tool()`, with env access injectable so tests don't touch
   real env vars.
5. **Settings** (new `src/settings.rs`): per-OS config dir (`$XDG_CONFIG_HOME`/`~/.config`,
   `~/Library/Application Support`, `%APPDATA%`). Tolerant parsing, atomic write.
6. **Toasts** (`src/ui/toasts.rs`): a queue in a foreground `egui::Area` at the bottom right, above
   the status bar. Kinds: info / success / error / install-progress. Auto-expire except errors and
   unfinished installs; dismiss button. Errors move off `message: Option<String>`. The status bar
   keeps the last neutral message ("Saved screenshot to …").
7. **Shortcuts** (`src/shortcuts.rs`): one table of `(Modifiers::COMMAND + key) → Action`, labels
   per OS ("⌘K" / "Ctrl K"), used for both tooltips and handling.
8. **Focus fix**: `raw_input_hook` surrenders widget focus every frame while a device is focused.
   That would make the new Send-text and filter fields impossible to type in. Exempt text edits
   and keep the Enter/Space protection for buttons.
9. **Icons** (`src/icons.rs`): add Search, Folder, Copy, Check, Close, Info, Menu, Sidebar, TileSize.
   Update `Icon::ALL` and the bounds test.
10. **Fonts** (`theme::install`): subset with `pyftsubset` via a committed `packaging/fonts/subset.sh`.
    Ranges: Basic Latin, Latin-1, General Punctuation (· … – —), × and the arrows/keys
    `← → ↑ ↓ ⌘ ⇧ ⌥ ⏎`. Commit the subsets + OFL licenses under `assets/fonts/`, load them with
    `FontDefinitions`, and drop eframe's `default_fonts` feature if the glyph test still passes.
    The glyph test must install our fonts (today it checks egui's defaults). Record binary size
    before/after in the PR.

## Phase 1 — Grid board (+ status bar, macOS chrome)

- **macOS chrome** (`main.rs`): `with_fullsize_content_view(true)`, hidden title, transparent
  title bar. Leave ~72 pt in the toolbar for the traffic lights. Empty toolbar space drags the
  window (`ViewportCommand::StartDrag`) and double-click zooms it.
- **Toolbar** (52 pt): wordmark, Grid/List segmented, spacer, **filter field** (search icon,
  "Filter devices", ⌘K hint and handler, Esc clears), **tile-size slider** (140–320, default 200,
  persisted, replaces `TILE_WIDTH`), Refresh, **Install APK** (primary → popover).
  Linux/Windows add the **Main menu** (Show sidebar, Set SDK folder…, Keyboard shortcuts,
  About). On macOS the sidebar toggle is an icon button.
- **Content header**: "Running devices" (17/600) + "N streaming · M paused off-screen" (from
  `visible`) + platform chips `All N / Android N / iOS N`. The chips show only when both
  platforms are present.
- **Tiles** (`tile()`): name with API ("Pixel 8 · API 35"), meta "Android 15 · Emulator" /
  "Android 14 · USB device" / "iOS 18.2 · Simulator", fps pill, transport tag
  `gRPC` / `adb · PNG` / `simctl · view only` / `starting`.
  **Hover quick bar** over the bezel bottom: Back / Home / Recents / Screenshot (iOS: Screenshot
  only). Its clicks must not open the tile.
  **Booting tile**: spinner, "Booting…", "<boot kind> · m:ss".
- **Grid keys**: 1–9 opens the nth visible tile when no text field has focus.
- **Status bar** (30 pt): SDK dot + root path (mono), "N devices · M running", last message,
  and a hint that depends on the view (Grid: "Click a tile to open · 1–9 jump to device";
  List: "Refreshes every 3 s · Drop an .apk anywhere…").

## Phase 2 — Focused device board

- **Header**: "‹ Grid" (Esc), "/", device switch button "Pixel 8 · API 35 ▾" (keep the
  dropdown, restyled), fps pill, spacer, keyboard pill
  **"Keyboard → device · Esc to release"**, and the screenshot icon. Stop moves to the inspector.
- **Filmstrip** (left, ~104 pt): one button per running device with a mini bezel showing its
  last frame, name, ⌘N label, accent ring on the current one. Clicking it focuses that device.
- **Stage**: dotted background from a tiny repeating texture (no per-dot painting), phone with
  an accent ring.
  **Control rail** (exists): tooltips show shortcuts, and the shortcuts work:
  ⌘← Back, ⌘⇧H Home, ⌘⇧R Recents, ⌘↓/⌘↑ volume, ⌘P Power, ⌘R Rotate, ⌘S Screenshot.
  `forward_keys` consumes these and never forwards them to the device.
- **Tap ripple**: on Tap/LongPress, store `(pos, Instant)`, paint a ring + dot for ~350 ms,
  and repaint while it is animating.
- **Zoom line** under the phone: "1080 × 2400 · shown at 58%" with **Fit** (today's behavior) /
  **100%** (1 device px = 1 physical px, pans in a ScrollArea, requests a full-size capture bucket) /
  **Pixel-perfect** (largest 1/n scale that fits, nearest filtering). Input mapping keeps using
  the drawn `img` rect.
- **Inspector** (right, ~300 pt):
  - DEVICE: Model, System, Screen (`screen_sizes`), Serial (mono), AVD (mono)
  - STREAM: fps / source / px bucket stat cards
  - SEND TEXT: field + Send (Enter sends, via `input::escape_text`), hint "Or just type…"
  - GESTURES: Click Tap · Drag Swipe · Hold, then drag · Right-click Long press · Scroll · Esc.
    This replaces the hint row under the phone.
  - **Stop emulator** (danger) at the bottom.
  iOS: same layout, "VIEW ONLY", no Send text, "Open in Simulator" + Shut down.
- **Narrow windows**: below ~900 pt the inspector collapses behind a "Details" toggle; below
  ~640 pt the filmstrip hides.

## Phase 3 — Device list board (+ install toast, Wipe confirm)

- **Header**: "Devices" (20/600) + "8 found · 4 running · 1 booting", **kind tabs**
  `All / Emulators / Physical / iOS` with counts, and **search** "Search name or serial"
  (shares the grid's filter text, so ⌘K works in both views).
- **Table**: 5 columns: Device (dot, display name, AVD id in mono), **System**, State, Serial,
  Actions. Group headers sit in a tinted band ("Android emulators · 7"). On narrow windows
  Serial hides first, then System. Update `list_columns_hide_serial_on_narrow_windows`.
- **Rows**:
  - Running: Open + Stop.
  - Booting: "Waiting for boot…" + Stop.
  - Stopped: split **Start**. The menu gets a first item, *Start — Resume from the saved
    snapshot*, then Cold boot, a separator, and *Wipe data and start…* (danger) →
    **confirmation dialog**.
  - Physical: state "Connected" (polled tone), sub-line "USB · screen at ~2 fps"
    ("Wi-Fi" for `host:port` serials). Unauthorized/offline show a hint to allow USB debugging.
- **iOS group**: rows on macOS with simctl; on macOS without Xcode, the existing info line;
  on **Linux/Windows a new info row**: "iOS simulators need macOS with Xcode. On a Mac they
  show up here automatically."
- **Install toast**: "Installing app-debug.apk · 2 of 4 done", dismiss, one row per device
  with Installed / Installing… (animated bar) / Failed + the real `INSTALL_FAILED_*` code.
  **"Retry with -d"** appears for `VERSION_DOWNGRADE`, and a plain Retry for other failures.
  `devices::install` gets a `downgrade` flag. `install_dropped` sends one `Msg` per device
  instead of a single summary.

## Phase 4 — Empty & setup board

- **No devices running**: copy "Start an emulator and its screen streams here live. Phones
  plugged in over USB show up too." "YOUR EMULATORS" + "All N in List" link, 2-column AVD cards
  (icon, display name, system line, Start).
  **ENVIRONMENT** checklist: Android SDK root, adb, emulator, iOS (simctl · N simulators) on
  macOS, each with a ✓/✗ icon.
- **SDK not found** (adb missing). Today the card only appears when *nothing* is found,
  simctl included. Rules: block the grid only when no tools at all are found; otherwise show
  the card in the empty state. Contents:
  - the list of searched places with their results (`sdk::diagnose()`)
  - **Choose SDK folder…** → inline path field + Use (checks for `platform-tools/adb`, saves to settings)
  - **Check again** (re-detect + rescan)
  - "No SDK yet?" with a command box `sdkmanager "platform-tools" "emulator"` and a **Copy**
    button (`ctx.copy_text`, then a toast)

## Phase 5 — Sidebar

- Left panel (~236 pt), grid view only, toggle persisted.
  - RUNNING · n: dot, name, fps meta; click to focus
  - ANDROID · STOPPED · n: ▶ Start
  - IOS · SHUT DOWN · n: ▶ Boot (macOS)
  - a dashed drop card at the bottom: "Drop an **.apk** anywhere to install on N Android devices"
- The section list scrolls when it is long.

## Phase 6 — Style-guide pass + docs

- Audit `theme.rs` and `widgets.rs` against board 5: token values (see My calls), control
  heights 28–36, rows 34, radii 5/8/14/20, shadows only on floating things.
- Pills must always show a dot and a word.
- Update `README.md` (shortcuts, zoom, install toast, SDK folder) and tick `docs/ROADMAP.md`.

---

## Critical files

`src/app.rs` → `src/ui/*.rs` · `src/devices.rs` · new `src/avd.rs`, `src/settings.rs`,
`src/shortcuts.rs` · `src/sdk.rs` · `src/capture/mod.rs` (fps sampling only) · `src/widgets.rs` ·
`src/icons.rs` · `src/theme.rs` (fonts, tokens) · `src/main.rs` (macOS viewport) · `Cargo.toml`
(drop `default_fonts` if possible) · `assets/fonts/`, `packaging/fonts/subset.sh` · `README.md`,
`docs/ROADMAP.md`.

Reuse: `widgets::{button, icon_button, segmented, status_pill, paint_pill, kbd, section_label,
bezel, split_button, menu_item, truncated}`, `grid::{columns, bucket}`, `input::{escape_text,
GestureTracker}`, `theme::Palette`/`Tone`, `devices::{start, stop, install, shell}`.

## Verification

- Every phase: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`.
- New unit tests:
  - parsing: `config.ini`/`<name>.ini`, getprop output (version, API, boot_completed), API → version table
  - logic: filter matching (name/serial, case-insensitive), fps meter math, settings round-trip and
    malformed input, `sdk::diagnose` with an injected env + temp dirs, `INSTALL_FAILED_*` parsing
  - UI rules: shortcut labels per OS, shortcut keys not forwarded, ripple expiry, Pixel-perfect scale
  - updated tests: list-column hiding, `Icon::ALL`, the glyph test on our fonts
- Size: `cargo build --release`, record `ls -l target/release/emugrid` before/after fonts and at
  the end. It must stay ≤ 8 MB (CI's Build workflow checks this too).
- Manual, macOS + Ubuntu, dark + light:
  - 3+ emulators (one cold-booting), a USB phone, iOS simulators (macOS)
  - every board: hover bar, slider, filter/⌘K, chips, 1–9
  - focused view: filmstrip, ⌘ shortcuts, Send text, ripple, zoom modes; typing still reaches the device
  - list: tabs/search, Wipe confirm, install with one forced downgrade failure → Retry with -d
  - empty state; SDK missing (`ANDROID_HOME` unset, adb off PATH) → set folder → Check again
  - narrow window (360 pt)

## Notes

- There is uncommitted in-progress work in `src/app.rs`, `src/capture/mod.rs`, `src/input.rs`,
  `src/input/adb.rs` and `README.md` (adb input worker, live touch, `Gesture::Cancel`). Build on
  top of it; never revert it. Ideally it gets committed before Phase 0's file split.
- Leave all changes uncommitted; the user commits.
