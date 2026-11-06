//! The egui window: a grid of live screens, a focused (interactive) device
//! view, and the device list with start / stop controls.

use crate::capture::{self, Manager, Source, grpc};
use crate::devices::{self, Boot, Device, Platform, State};
use crate::sdk::Sdk;
use crate::{grid, input};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Vec2};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

const REFRESH_EVERY: Duration = Duration::from_secs(3);
const SCROLL_SETTLE: Duration = Duration::from_millis(150);
const TILE_WIDTH: f32 = 190.0;
const GAP: f32 = 12.0;
/// Phone-shaped screen box used before the first frame arrives.
const DEFAULT_ASPECT: f32 = 2400.0 / 1080.0;

const GREEN: Color32 = Color32::from_rgb(60, 180, 90);
const AMBER: Color32 = Color32::from_rgb(220, 160, 40);

#[derive(Clone, Copy, PartialEq)]
enum View {
    Grid,
    List,
}

/// Results coming back from background threads.
enum Msg {
    Scan(Vec<Device>, HashMap<String, grpc::Endpoint>),
    Status(String),
    /// `None` when `wm size` failed; it is asked again.
    ScreenSize(String, Option<(u32, u32)>),
}

/// Things the user asked for while the UI was being drawn.
enum Action {
    Focus(String),
    Unfocus,
    View(View),
    Refresh,
    Start(String, Boot),
    Stop(String),
    IosBoot(String),
    IosShutdown(String),
    IosOpen(String),
    Rotate(String),
    Screenshot(String),
    /// `adb shell <cmd>` on a serial, run in order with other input.
    Shell(String, String),
}

struct Screen {
    tex: egui::TextureHandle,
    size: [usize; 2],
}

impl Screen {
    fn aspect(&self) -> f32 {
        self.size[1] as f32 / self.size[0].max(1) as f32
    }
}

pub struct EmuGrid {
    sdk: Sdk,
    devices: Vec<Device>,
    endpoints: HashMap<String, grpc::Endpoint>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    scanning: bool,
    last_scan: Option<Instant>,
    message: Option<String>,
    view: View,
    /// Serial of the device shown large.
    focused: Option<String>,
    capture: Manager,
    screens: HashMap<String, Screen>,
    /// Real screen size per Android serial (`wm size`), for mapping taps.
    screen_sizes: HashMap<String, (u32, u32)>,
    input_tx: Sender<(String, String)>,
    /// Tiles drawn on screen this frame; everything else pauses.
    visible: HashSet<String>,
    /// Accumulated scroll (points) and when it last changed.
    scroll: (f32, Option<Instant>),
    gestures: input::GestureTracker,
    /// Tab presses taken from egui, to send to the focused device.
    pending_tabs: usize,
    /// A live finger held down on a device: serial and last device position.
    finger: Option<(String, (u32, u32))>,
    /// Serials with a `wm size` lookup in flight.
    size_pending: HashSet<String>,
}

impl EmuGrid {
    pub fn new(ctx: &egui::Context) -> Self {
        let sdk = Sdk::detect();
        let (tx, rx) = mpsc::channel();

        // One thread runs input commands in order, so taps and text never reorder.
        let (input_tx, input_rx) = mpsc::channel::<(String, String)>();
        let input_sdk = sdk.clone();
        thread::spawn(move || {
            let mut queue = std::collections::VecDeque::new();
            while let Ok(first) = input_rx.recv() {
                queue.push_back(first);
                while let Some((serial, cmd)) = input::next_command(&mut queue, &input_rx) {
                    let _ = devices::shell(&input_sdk, &serial, &cmd);
                }
            }
        });

        Self {
            sdk,
            devices: Vec::new(),
            endpoints: HashMap::new(),
            tx,
            rx,
            scanning: false,
            last_scan: None,
            message: None,
            view: View::Grid,
            focused: None,
            capture: Manager::new(ctx.clone()),
            screens: HashMap::new(),
            screen_sizes: HashMap::new(),
            input_tx,
            visible: HashSet::new(),
            scroll: (0.0, None),
            gestures: input::GestureTracker::default(),
            pending_tabs: 0,
            size_pending: HashSet::new(),
            finger: None,
        }
    }

    /// Scans run on a background thread so the UI never blocks on adb.
    fn scan(&mut self, ctx: &egui::Context) {
        if self.scanning {
            return;
        }
        self.scanning = true;
        self.last_scan = Some(Instant::now());
        let (sdk, tx, ctx) = (self.sdk.clone(), self.tx.clone(), ctx.clone());
        thread::spawn(move || {
            let _ = tx.send(Msg::Scan(devices::scan(&sdk), grpc::discover()));
            ctx.request_repaint();
        });
    }

    /// Run `job` off the UI thread and show its result in the status bar.
    fn background(
        &self,
        ctx: &egui::Context,
        job: impl FnOnce(&Sdk) -> Result<String, String> + Send + 'static,
    ) {
        let (sdk, tx, ctx) = (self.sdk.clone(), self.tx.clone(), ctx.clone());
        thread::spawn(move || {
            let msg = job(&sdk).unwrap_or_else(|e| format!("Error: {e}"));
            let _ = tx.send(Msg::Status(msg));
            ctx.request_repaint();
        });
    }

    fn on_scan(&mut self, list: Vec<Device>, endpoints: HashMap<String, grpc::Endpoint>) {
        self.devices = list;
        self.endpoints = endpoints;
        self.scanning = false;

        let mut wanted = Vec::new();
        for d in &self.devices {
            let Some(serial) = d.serial() else { continue };
            let source = match (d.platform, &self.sdk.adb) {
                (Platform::Ios, _) => Source::Simctl,
                (Platform::Android, Some(adb)) => match self.endpoints.get(serial) {
                    Some(ep) => Source::Grpc {
                        ep: ep.clone(),
                        adb: adb.clone(),
                    },
                    None => Source::Adb { adb: adb.clone() },
                },
                (Platform::Android, None) => continue,
            };
            wanted.push((serial.to_string(), source));
        }
        let alive: HashSet<String> = wanted.iter().map(|(s, _)| s.clone()).collect();
        self.capture.sync(wanted);
        self.screens.retain(|s, _| alive.contains(s));
        self.screen_sizes.retain(|s, _| alive.contains(s));
        if self.focused.as_ref().is_some_and(|f| !alive.contains(f)) {
            self.focused = None;
        }
    }

    /// Look up an Android device's real screen size (for mapping taps) until
    /// it is known. Failed lookups are retried after a pause.
    fn ensure_screen_size(&mut self, ctx: &egui::Context, serial: &str) {
        if self.screen_sizes.contains_key(serial) || !self.size_pending.insert(serial.into()) {
            return;
        }
        let (sdk, tx, s, ctx) = (
            self.sdk.clone(),
            self.tx.clone(),
            serial.to_string(),
            ctx.clone(),
        );
        thread::spawn(move || {
            let size = devices::shell(&sdk, &s, "wm size")
                .ok()
                .and_then(|o| input::parse_wm_size(&o));
            if size.is_none() {
                thread::sleep(Duration::from_secs(2));
            }
            let _ = tx.send(Msg::ScreenSize(s, size));
            ctx.request_repaint();
        });
    }

    /// Never leave a finger pressed on a device we stop looking at.
    fn lift_finger(&mut self) {
        self.gestures = input::GestureTracker::default();
        if let Some((serial, (x, y))) = self.finger.take() {
            let _ = self
                .input_tx
                .send((serial, format!("input motionevent CANCEL {x} {y}")));
        }
    }

    /// Name shown for a device; emulators often share a model name, so add
    /// the serial when the name alone is ambiguous.
    fn label(&self, d: &Device) -> String {
        let same = self.running().filter(|o| o.name == d.name).count();
        match d.serial() {
            Some(serial) if same > 1 && d.platform == Platform::Android => {
                format!("{} · {serial}", d.name)
            }
            _ => d.name.clone(),
        }
    }

    fn running(&self) -> impl Iterator<Item = &Device> {
        self.devices.iter().filter(|d| d.serial().is_some())
    }

    fn device(&self, serial: &str) -> Option<&Device> {
        self.devices.iter().find(|d| d.serial() == Some(serial))
    }

    fn apply(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::Focus(serial) => {
                self.lift_finger();
                self.scroll = (0.0, None);
                self.gestures = input::GestureTracker::default();
                self.focused = Some(serial);
                self.view = View::Grid;
            }
            Action::Unfocus => {
                self.lift_finger();
                self.focused = None;
            }
            Action::View(v) => {
                self.lift_finger();
                self.view = v;
                self.focused = None;
            }
            Action::Refresh => self.last_scan = None,
            Action::Start(avd, boot) => {
                let how = match boot {
                    Boot::Normal => "Starting",
                    Boot::Cold => "Cold booting",
                    Boot::Wipe => "Wiping and starting",
                };
                self.message = Some(
                    devices::start(&self.sdk, &avd, boot)
                        .map(|_| format!("{how} {avd}…"))
                        .unwrap_or_else(|e| format!("Error: {e}")),
                );
                self.last_scan = None;
            }
            Action::Stop(serial) => {
                self.background(ctx, move |sdk| {
                    devices::stop(sdk, &serial).map(|_| format!("Stopping {serial}…"))
                });
                self.last_scan = None;
            }
            Action::IosBoot(udid) => {
                self.message = Some("Booting simulator…".into());
                self.background(ctx, move |_| {
                    devices::ios_boot(&udid).map(|_| "Simulator booted".into())
                });
            }
            Action::IosShutdown(udid) => {
                self.background(ctx, move |_| {
                    devices::ios_shutdown(&udid).map(|_| "Simulator shut down".into())
                });
            }
            Action::IosOpen(udid) => {
                if let Err(e) = devices::ios_open(&udid) {
                    self.message = Some(format!("Error: {e}"));
                }
            }
            Action::Rotate(serial) => {
                self.background(ctx, move |sdk| {
                    devices::rotate(sdk, &serial).map(|_| "Rotated".into())
                });
            }
            Action::Screenshot(serial) => {
                if let Some(d) = self.device(&serial).cloned() {
                    self.background(ctx, move |sdk| {
                        devices::save_screenshot(sdk, &d).map(|p| format!("Saved {}", p.display()))
                    });
                }
            }
            Action::Shell(serial, cmd) => {
                let _ = self.input_tx.send((serial, cmd));
            }
        }
    }

    /// Install dropped `.apk` files on every running Android device.
    fn install_dropped(&mut self, ctx: &egui::Context, apks: Vec<PathBuf>) {
        let serials: Vec<String> = self
            .running()
            .filter(|d| d.platform == Platform::Android)
            .filter_map(|d| d.serial().map(String::from))
            .collect();
        if serials.is_empty() {
            self.message = Some("No running Android devices to install on.".into());
            return;
        }
        for apk in apks {
            let name = apk
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.message = Some(format!("Installing {name} on {} devices…", serials.len()));
            let serials = serials.clone();
            self.background(ctx, move |sdk| {
                let handles: Vec<_> = serials
                    .into_iter()
                    .map(|s| {
                        let (sdk, apk) = (sdk.clone(), apk.clone());
                        thread::spawn(move || devices::install(&sdk, &s, &apk).map_err(|e| (s, e)))
                    })
                    .collect();
                let total = handles.len();
                let failed: Vec<String> = handles
                    .into_iter()
                    .filter_map(|h| h.join().ok()?.err())
                    .map(|(s, e)| format!("{s}: {e}"))
                    .collect();
                if failed.is_empty() {
                    Ok(format!("Installed {name} on {total} devices"))
                } else {
                    Err(format!(
                        "{name} failed on {}/{total} — {}",
                        failed.len(),
                        failed.join("; ")
                    ))
                }
            });
        }
    }

    // ---------------------------------------------------------------- views

    fn grid_view(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let running: Vec<Device> = self.running().cloned().collect();
        if running.is_empty() {
            self.empty_grid(ui, actions);
            return;
        }
        let ppp = ui.ctx().pixels_per_point();
        egui::ScrollArea::vertical().show(ui, |ui| {
            let width = ui.available_width();
            let cols = grid::columns(width, TILE_WIDTH, GAP, running.len());
            let tile_w = ((width - GAP * (cols - 1) as f32) / cols as f32).min(TILE_WIDTH * 1.6);
            for row in running.chunks(cols) {
                // Each row is as tall as its tallest screen.
                let screen_h = row
                    .iter()
                    .map(|d| {
                        let aspect = d
                            .serial()
                            .and_then(|s| self.screens.get(s))
                            .map_or(DEFAULT_ASPECT, Screen::aspect);
                        (tile_w - 8.0) * aspect.clamp(0.4, 2.4)
                    })
                    .fold(0.0, f32::max);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = GAP;
                    for d in row {
                        if let Some(a) = self.tile(ui, d, Vec2::new(tile_w, screen_h + 46.0), ppp) {
                            actions.push(a);
                        }
                    }
                });
                ui.add_space(GAP);
            }
        });
    }

    fn tile(&mut self, ui: &mut egui::Ui, d: &Device, size: Vec2, ppp: f32) -> Option<Action> {
        let serial = d.serial()?.to_string();
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        if !ui.is_rect_visible(rect) {
            return None;
        }
        self.visible.insert(serial.clone());
        let visuals = ui.visuals();
        let painter = ui.painter_at(rect);
        let stroke = if resp.hovered() {
            visuals.selection.stroke
        } else {
            visuals.widgets.noninteractive.bg_stroke
        };
        painter.rect(
            rect,
            8.0,
            visuals.faint_bg_color,
            stroke,
            egui::StrokeKind::Inside,
        );

        // Title.
        let text = visuals.text_color();
        painter.text(
            rect.left_top() + Vec2::new(10.0, 8.0),
            Align2::LEFT_TOP,
            self.label(d),
            FontId::proportional(13.0),
            text,
        );

        // Screen.
        let screen_box = Rect::from_min_max(
            rect.left_top() + Vec2::new(4.0, 26.0),
            rect.right_bottom() - Vec2::new(4.0, 20.0),
        );
        let control = self.capture.control(&serial);
        if let Some(c) = control {
            c.size.store(
                grid::bucket(screen_box.height().max(screen_box.width()) * ppp),
                Ordering::Relaxed,
            );
        }
        let img = self.paint_screen(&painter, &serial, screen_box);

        // Status line.
        let (label, color) = status_badge(d, control.map(|c| c.live.load(Ordering::Relaxed)));
        painter.text(
            Pos2::new(rect.left() + 10.0, rect.bottom() - 10.0),
            Align2::LEFT_CENTER,
            format!("⚫ {label}"),
            FontId::proportional(11.0),
            color,
        );
        if img.is_none() {
            painter.text(
                screen_box.center(),
                Align2::CENTER_CENTER,
                "waiting for screen…",
                FontId::proportional(12.0),
                visuals.weak_text_color(),
            );
        }
        resp.on_hover_text("Click to open")
            .clicked()
            .then_some(Action::Focus(serial))
    }

    /// Draw the latest frame fitted into `area`. Returns where it was drawn.
    fn paint_screen(&self, painter: &egui::Painter, serial: &str, area: Rect) -> Option<Rect> {
        let screen = self.screens.get(serial)?;
        let [w, h] = screen.size.map(|v| v as f32);
        let scale = (area.width() / w).min(area.height() / h);
        let rect = Rect::from_center_size(area.center(), Vec2::new(w, h) * scale);
        painter.image(
            screen.tex.id(),
            rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
        Some(rect)
    }

    fn empty_grid(&self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.heading("No running devices");
            ui.label("Start an emulator and its screen shows up here.");
            ui.add_space(12.0);
            for d in self.devices.iter().filter(|d| d.avd.is_some()).take(8) {
                if ui.button(format!("▶ {}", d.name)).clicked() {
                    actions.push(Action::Start(
                        d.avd.clone().unwrap_or_default(),
                        Boot::Normal,
                    ));
                }
            }
            if self.devices.iter().any(|d| d.platform == Platform::Ios) {
                ui.add_space(8.0);
                if ui.link("iOS simulators are in the List view").clicked() {
                    actions.push(Action::View(View::List));
                }
            }
        });
    }

    fn focused_view(&mut self, ui: &mut egui::Ui, serial: &str, actions: &mut Vec<Action>) {
        let Some(d) = self.device(serial).cloned() else {
            actions.push(Action::Unfocus);
            return;
        };
        let android = d.platform == Platform::Android;
        if android {
            self.ensure_screen_size(ui.ctx(), serial);
        }
        let shell = |cmd: &str| Action::Shell(serial.to_string(), cmd.to_string());

        ui.horizontal_wrapped(|ui| {
            if ui.button("⬅ Grid").on_hover_text("Esc").clicked() {
                actions.push(Action::Unfocus);
            }
            let current = self.label(&d);
            let others: Vec<(String, String)> = self
                .running()
                .filter_map(|o| Some((o.serial()?.to_string(), self.label(o))))
                .collect();
            egui::ComboBox::from_id_salt("device-switch")
                .selected_text(egui::RichText::new(current).strong())
                .width(240.0)
                .show_ui(ui, |ui| {
                    for (s, name) in others {
                        if ui.selectable_label(s == serial, name).clicked() && s != serial {
                            actions.push(Action::Focus(s));
                        }
                    }
                })
                .response
                .on_hover_text("Switch device");
            let live = self
                .capture
                .control(serial)
                .map(|c| c.live.load(Ordering::Relaxed));
            let (label, color) = status_badge(&d, live);
            ui.colored_label(color, format!("⚫ {label}"));
            ui.separator();
            if android {
                for (label, key, tip) in [
                    ("◀ Back", 4, "Back"),
                    ("🏠 Home", 3, "Home"),
                    ("▣ Recents", 187, "Recent apps"),
                    ("🔉", 25, "Volume down"),
                    ("🔊", 24, "Volume up"),
                    ("⚡ Power", 26, "Power button"),
                ] {
                    if ui.button(label).on_hover_text(tip).clicked() {
                        actions.push(shell(&format!("input keyevent {key}")));
                    }
                }
                if d.is_emulator
                    && ui
                        .button("⟳ Rotate")
                        .on_hover_text("Rotate the emulator")
                        .clicked()
                {
                    actions.push(Action::Rotate(serial.to_string()));
                }
            } else if let Some(udid) = &d.udid
                && ui.button("Open in Simulator").clicked()
            {
                actions.push(Action::IosOpen(udid.clone()));
            }
            if ui
                .button("📷 Screenshot")
                .on_hover_text("Save screenshot to Desktop")
                .clicked()
            {
                actions.push(Action::Screenshot(serial.to_string()));
            }
            if d.is_emulator && ui.button("■ Stop").clicked() {
                actions.push(match &d.udid {
                    Some(udid) => Action::IosShutdown(udid.clone()),
                    None => Action::Stop(serial.to_string()),
                });
            }
        });
        ui.add_space(4.0);

        let hint_h = 18.0;
        let area = ui.available_rect_before_wrap();
        let area = Rect::from_min_max(area.min, area.max - Vec2::new(0.0, hint_h));
        let (rect, resp) = ui.allocate_exact_size(area.size(), Sense::click_and_drag());
        let ppp = ui.ctx().pixels_per_point();
        if let Some(c) = self.capture.control(serial) {
            c.size.store(
                grid::bucket(rect.height().max(rect.width()) * ppp),
                Ordering::Relaxed,
            );
        }
        self.visible.insert(serial.to_string());
        let painter = ui.painter_at(rect);
        let img = self.paint_screen(&painter, serial, rect);
        if img.is_none() {
            ui.put(rect, egui::Spinner::new());
        }
        ui.vertical_centered(|ui| {
            ui.small(if android {
                "click = tap · drag = swipe · hold = long press, then drag to move · type to send keys · Esc = back"
            } else {
                "View only — use “Open in Simulator” to interact"
            });
        });

        // Input (Android only).
        let (Some(img), true) = (img, android) else {
            return;
        };
        let Some(&screen) = self.screen_sizes.get(serial) else {
            return;
        };
        let landscape = self
            .screens
            .get(serial)
            .is_some_and(|s| s.size[0] > s.size[1]);
        let to_dev = |p: Pos2| {
            let uv = (p - img.min) / img.size();
            input::to_device((uv.x, uv.y), screen, landscape)
        };
        let ctx = ui.ctx();
        match self.gestures.update(&resp) {
            Some(input::Gesture::Tap(p)) if img.contains(p) => {
                let (x, y) = to_dev(p);
                actions.push(shell(&format!("input tap {x} {y}")));
            }
            Some(input::Gesture::LongPress(p)) if img.contains(p) => {
                let (x, y) = to_dev(p);
                actions.push(shell(&format!("input swipe {x} {y} {x} {y} 800")));
            }
            Some(input::Gesture::Swipe(a, b, ms)) if img.contains(a) => {
                let ((x1, y1), (x2, y2)) = (to_dev(a), to_dev(b));
                actions.push(shell(&format!("input swipe {x1} {y1} {x2} {y2} {ms}")));
            }
            Some(input::Gesture::Down(p)) if img.contains(p) => {
                let (x, y) = to_dev(p);
                self.finger = Some((serial.to_string(), (x, y)));
                actions.push(shell(&format!("input motionevent DOWN {x} {y}")));
            }
            Some(input::Gesture::Move(p)) if self.finger.is_some() => {
                let (x, y) = to_dev(p);
                self.finger = Some((serial.to_string(), (x, y)));
                actions.push(shell(&format!("input motionevent MOVE {x} {y}")));
            }
            Some(input::Gesture::Up(p)) if self.finger.is_some() => {
                let (x, y) = to_dev(p);
                self.finger = None;
                actions.push(shell(&format!("input motionevent UP {x} {y}")));
            }
            _ => {}
        }
        // Scroll wheel / trackpad -> one swipe once the gesture settles.
        if resp.hovered() {
            let delta: f32 = ctx.input(|i| {
                i.raw
                    .events
                    .iter()
                    .filter_map(|e| match e {
                        egui::Event::MouseWheel { unit, delta, .. } => Some(match unit {
                            egui::MouseWheelUnit::Point => delta.y,
                            egui::MouseWheelUnit::Line => delta.y * 40.0,
                            egui::MouseWheelUnit::Page => delta.y * img.height(),
                        }),
                        _ => None,
                    })
                    .sum()
            });
            if delta != 0.0 {
                self.scroll.0 += delta;
                self.scroll.1 = Some(Instant::now());
                ctx.request_repaint_after(SCROLL_SETTLE);
            }
        }
        if self.scroll.1.is_some_and(|t| t.elapsed() >= SCROLL_SETTLE) {
            let c = img.center();
            let dy = self
                .scroll
                .0
                .clamp(-img.height() * 0.45, img.height() * 0.45);
            self.scroll = (0.0, None);
            if dy.abs() > 4.0 {
                let ((x1, y1), (x2, y2)) = (
                    to_dev(c - Vec2::new(0.0, dy / 2.0)),
                    to_dev(c + Vec2::new(0.0, dy / 2.0)),
                );
                actions.push(shell(&format!("input swipe {x1} {y1} {x2} {y2} 150")));
            }
        }
    }

    /// Keyboard -> device, while a device is focused.
    fn forward_keys(&mut self, ctx: &egui::Context, serial: &str, actions: &mut Vec<Action>) {
        let tabs = std::mem::take(&mut self.pending_tabs);
        if ctx.text_edit_focused() {
            return;
        }
        let android = self
            .device(serial)
            .is_some_and(|d| d.platform == Platform::Android);
        let events = ctx.input(|i| i.events.clone());
        let mut text = String::new();
        for e in events {
            match e {
                egui::Event::Key {
                    key: egui::Key::Escape,
                    pressed: true,
                    ..
                } => actions.push(Action::Unfocus),
                egui::Event::Text(t) | egui::Event::Paste(t) if android => text.push_str(&t),
                egui::Event::Key {
                    key, pressed: true, ..
                } if android => {
                    if let Some(code) = input::keycode(key) {
                        if !text.is_empty() {
                            actions.push(Action::Shell(
                                serial.into(),
                                format!("input text {}", input::escape_text(&text)),
                            ));
                            text.clear();
                        }
                        actions.push(Action::Shell(
                            serial.into(),
                            format!("input keyevent {code}"),
                        ));
                    }
                }
                _ => {}
            }
        }
        if !text.is_empty() {
            actions.push(Action::Shell(
                serial.into(),
                format!("input text {}", input::escape_text(&text)),
            ));
        }
        if android {
            for _ in 0..tabs {
                actions.push(Action::Shell(serial.into(), "input keyevent 61".into()));
            }
        }
    }

    fn list_view(&self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            let android: Vec<&Device> = self
                .devices
                .iter()
                .filter(|d| d.platform == Platform::Android)
                .collect();
            if android.is_empty() && !self.scanning {
                ui.label("No Android emulators or devices found.");
            }
            for d in android {
                device_row(ui, d, actions);
            }

            let ios: Vec<&Device> = self
                .devices
                .iter()
                .filter(|d| d.platform == Platform::Ios)
                .collect();
            if !ios.is_empty() {
                ui.add_space(8.0);
                let booted = ios.iter().filter(|d| d.serial().is_some()).count();
                egui::CollapsingHeader::new(format!(
                    "iOS simulators · {} · {booted} booted",
                    ios.len()
                ))
                .default_open(booted > 0 || ios.len() <= 6)
                .show(ui, |ui| {
                    for d in ios {
                        device_row(ui, d, actions);
                    }
                });
            } else if cfg!(target_os = "macos") && !self.sdk.simctl {
                ui.add_space(8.0);
                ui.small("iOS simulators: install Xcode to see and control them here.");
            }
        });
    }
}

/// Short status for a running device: how its screen is being captured.
fn status_badge(d: &Device, live: Option<bool>) -> (String, Color32) {
    match (&d.state, live) {
        (State::Running { .. }, Some(true)) => ("live".into(), GREEN),
        (State::Running { .. }, Some(false)) => {
            let kind = match (d.platform, d.is_emulator) {
                (Platform::Ios, _) => "simulator",
                (_, true) => "emulator",
                (_, false) => "device",
            };
            (format!("{kind} · ~2 fps"), AMBER)
        }
        (State::Running { .. }, None) => ("starting".into(), AMBER),
        (State::Other { status, .. }, _) => (status.clone(), AMBER),
        (State::Stopped, _) => ("stopped".into(), Color32::GRAY),
    }
}

fn device_row(ui: &mut egui::Ui, d: &Device, actions: &mut Vec<Action>) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let (dot, color) = match &d.state {
                State::Running { .. } => ("⚫", GREEN),
                State::Other { .. } => ("⚫", AMBER),
                State::Stopped => ("○", ui.visuals().weak_text_color()),
            };
            ui.colored_label(color, dot);
            ui.vertical(|ui| {
                ui.strong(&d.name);
                let kind = match (d.platform, d.is_emulator) {
                    (Platform::Ios, _) => {
                        format!("iOS simulator · {}", d.runtime.as_deref().unwrap_or("iOS"))
                    }
                    (_, true) => "Emulator".into(),
                    (_, false) => "Physical device".into(),
                };
                let detail = match &d.state {
                    State::Running { serial } if d.platform == Platform::Ios => {
                        format!("{kind} · booted")
                    }
                    State::Running { serial } => format!("{kind} · running · {serial}"),
                    State::Other { serial, status } => format!("{kind} · {status} · {serial}"),
                    State::Stopped => format!("{kind} · stopped"),
                };
                ui.small(detail);
            });
            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| match (&d.state, d.platform) {
                    (State::Stopped, Platform::Android) => {
                        if let Some(avd) = &d.avd {
                            ui.menu_button("…", |ui| {
                                if ui.button("Cold boot").clicked() {
                                    actions.push(Action::Start(avd.clone(), Boot::Cold));
                                }
                                if ui.button("Wipe data and start").clicked() {
                                    actions.push(Action::Start(avd.clone(), Boot::Wipe));
                                }
                            });
                            if ui.button("▶ Start").clicked() {
                                actions.push(Action::Start(avd.clone(), Boot::Normal));
                            }
                        }
                    }
                    (State::Stopped, Platform::Ios) => {
                        if let Some(udid) = &d.udid
                            && ui.button("▶ Boot").clicked()
                        {
                            actions.push(Action::IosBoot(udid.clone()));
                        }
                    }
                    (State::Running { serial }, platform) => {
                        if d.is_emulator && ui.button("■ Stop").clicked() {
                            actions.push(match (platform, &d.udid) {
                                (Platform::Ios, Some(udid)) => Action::IosShutdown(udid.clone()),
                                _ => Action::Stop(serial.clone()),
                            });
                        }
                        if ui.button("View").clicked() {
                            actions.push(Action::Focus(serial.clone()));
                        }
                    }
                    _ => {}
                },
            );
        });
    });
}

impl eframe::App for EmuGrid {
    /// Runs even while the window is hidden (when `ui` is skipped): pause all
    /// streams then, so hidden tiles cost nothing.
    /// While a device is focused the keyboard belongs to it: take Tab before
    /// egui turns it into widget focus, and make sure no button holds focus
    /// (Enter / Space would press it).
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        if self.focused.is_none() || self.view != View::Grid {
            return;
        }
        self.pending_tabs += input::take_tabs(&mut raw.events);
        ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input(|i| i.viewport().visible() == Some(false)) {
            for (_, c) in self.capture.controls() {
                c.active.store(false, Ordering::Relaxed);
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = &ui.ctx().clone();
        let full = ui.max_rect();

        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Scan(list, eps) => self.on_scan(list, eps),
                Msg::Status(s) => {
                    self.message = Some(s);
                    self.last_scan = None;
                }
                Msg::ScreenSize(serial, size) => {
                    self.size_pending.remove(&serial);
                    if let Some(size) = size {
                        self.screen_sizes.insert(serial, size);
                    }
                }
            }
        }
        let frames: Vec<_> = self.capture.frames().collect();
        for (serial, f) in frames {
            let size = [f.width as usize, f.height as usize];
            let image = egui::ColorImage::from_rgb(size, &f.rgb);
            match self.screens.get_mut(&serial) {
                Some(s) => {
                    s.tex.set(image, egui::TextureOptions::LINEAR);
                    s.size = size;
                }
                None => {
                    let tex = ctx.load_texture(&serial, image, egui::TextureOptions::LINEAR);
                    self.screens.insert(serial, Screen { tex, size });
                }
            }
        }

        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("apk")))
                .collect()
        });
        if !dropped.is_empty() {
            self.install_dropped(ctx, dropped);
        }

        if self.last_scan.is_none_or(|t| t.elapsed() >= REFRESH_EVERY) {
            self.scan(ctx);
        }
        ctx.request_repaint_after(REFRESH_EVERY);

        let mut actions = Vec::new();
        self.visible.clear();

        egui::Panel::top("top").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("emugrid");
                ui.add_space(8.0);
                let grid_on = self.view == View::Grid;
                if ui.selectable_label(grid_on, "⊞ Grid").clicked() {
                    actions.push(Action::View(View::Grid));
                }
                if ui.selectable_label(!grid_on, "☰ List").clicked() {
                    actions.push(Action::View(View::List));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("⟳ Refresh").clicked() {
                        actions.push(Action::Refresh);
                    }
                    if self.scanning {
                        ui.spinner();
                    }
                });
            });
        });

        egui::Panel::bottom("status").show(ui, |ui| {
            let running = self.running().count();
            ui.horizontal(|ui| {
                ui.label(format!(
                    "{} devices · {running} running",
                    self.devices.len()
                ));
                if let Some(msg) = &self.message {
                    ui.separator();
                    ui.label(msg);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.small("Drop an .apk to install it everywhere")
                        .on_hover_text("Installs on every running Android device");
                });
            });
        });

        egui::CentralPanel::default_margins().show(ui, |ui| {
            if self.sdk.adb.is_none() && self.sdk.emulator.is_none() && !self.sdk.simctl {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "Android SDK not found. Set ANDROID_HOME or put adb and emulator on your PATH.",
                );
                return;
            }
            match (self.view, self.focused.clone()) {
                (View::Grid, Some(serial)) => {
                    self.focused_view(ui, &serial, &mut actions);
                    self.forward_keys(ctx, &serial, &mut actions);
                }
                (View::Grid, None) => self.grid_view(ui, &mut actions),
                (View::List, _) => self.list_view(ui, &mut actions),
            }
        });

        // Drag-and-drop hint.
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if hovering {
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::unique("emugrid-drop"),
            ));
            painter.rect_filled(full, 0.0, Color32::from_black_alpha(170));
            painter.text(
                full.center(),
                Align2::CENTER_CENTER,
                "Drop .apk to install on all running Android devices",
                FontId::proportional(18.0),
                Color32::WHITE,
            );
        }

        for a in actions {
            self.apply(ctx, a);
        }

        // Only stream what is on screen; nothing at all while minimized.
        let minimized = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
        for (serial, c) in self.capture.controls() {
            let on = !minimized && self.visible.contains(serial);
            c.active.store(on, Ordering::Relaxed);
            let fps = if self.focused.as_deref() == Some(serial.as_str()) {
                capture::FOCUSED_FPS
            } else {
                capture::TILE_FPS
            };
            c.max_fps.store(fps, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serials get reused (`emulator-5554`), so a stopped device's screen
    /// size must not be used for the next device that takes its serial.
    #[test]
    fn forgets_screen_size_when_device_goes_away() {
        let mut app = EmuGrid::new(&egui::Context::default());
        app.screen_sizes
            .insert("emulator-5554".into(), (1080, 2400));
        app.on_scan(Vec::new(), HashMap::new());
        assert!(app.screen_sizes.is_empty());
    }

    /// Every non-ASCII character in the UI's text must exist in the bundled
    /// fonts, or it draws as an empty box.
    #[test]
    fn ui_symbols_have_glyphs() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |_| {});
        out.textures_delta.clear();
        let source = include_str!("app.rs");
        let mut chars: Vec<char> = source.chars().filter(|c| !c.is_ascii()).collect();
        chars.sort();
        chars.dedup();
        let missing: Vec<char> = ctx.fonts_mut(|f| {
            chars
                .into_iter()
                .filter(|&c| !f.has_glyph(&FontId::proportional(14.0), c))
                .collect()
        });
        assert!(missing.is_empty(), "no glyph for {missing:?}");
    }

    /// Clicking the middle of the focused screen sends a tap there.
    #[test]
    fn click_on_focused_screen_sends_tap() {
        use egui::{Event, PointerButton, RawInput};
        let ctx = egui::Context::default();
        let mut app = EmuGrid::new(&ctx);
        app.devices = vec![Device {
            name: "phone".into(),
            platform: Platform::Android,
            is_emulator: true,
            state: State::Running {
                serial: "emulator-5554".into(),
            },
            avd: None,
            runtime: None,
            udid: None,
        }];
        let tex = ctx.load_texture(
            "s",
            egui::ColorImage::from_rgb([108, 240], &vec![0; 108 * 240 * 3]),
            egui::TextureOptions::LINEAR,
        );
        app.screens.insert(
            "emulator-5554".into(),
            Screen {
                tex,
                size: [108, 240],
            },
        );
        app.screen_sizes
            .insert("emulator-5554".into(), (1080, 2400));
        app.focused = Some("emulator-5554".into());

        let center = Pos2::new(400.0, 330.0);
        let steps: Vec<(f64, Vec<Event>)> = vec![
            (0.0, vec![Event::PointerMoved(center)]),
            (
                0.1,
                vec![Event::PointerButton {
                    pos: center,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                }],
            ),
            (
                0.2,
                vec![Event::PointerButton {
                    pos: center,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                }],
            ),
            (0.3, vec![]),
        ];
        let mut sent = Vec::new();
        for (t, events) in steps {
            let raw = RawInput {
                time: Some(t),
                events,
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 700.0))),
                ..Default::default()
            };
            let mut out = ctx.run_ui(raw, |ui| {
                let mut actions = Vec::new();
                app.focused_view(ui, "emulator-5554", &mut actions);
                for a in actions {
                    if let Action::Shell(_, cmd) = a {
                        sent.push(cmd);
                    }
                }
            });
            out.textures_delta.clear();
        }
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert!(sent[0].starts_with("input tap "), "{sent:?}");
    }
}
