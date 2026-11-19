//! The egui window: a grid of live screens, a focused (interactive) device
//! view, and the device list with start / stop controls.

use crate::capture::{self, Manager, Source, grpc};
use crate::devices::{self, Boot, Device, Platform, State};
use crate::icons::{self, Icon};
use crate::sdk::Sdk;
use crate::theme::{self, RADIUS_CARD, Tone};
use crate::widgets::{self, Kind};
use crate::{grid, input};
use eframe::egui::{self, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

const REFRESH_EVERY: Duration = Duration::from_secs(3);
const SCROLL_SETTLE: Duration = Duration::from_millis(150);
const TILE_WIDTH: f32 = 200.0;
/// Padding inside a tile card.
const TILE_PAD: f32 = 12.0;
const TILE_HEADER: f32 = 40.0;
const TILE_FOOTER: f32 = 24.0;
/// Everything in a tile that is not the screen: padding, header, footer,
/// gaps and the phone frame.
const TILE_CHROME: f32 = TILE_PAD * 2.0 + TILE_HEADER + TILE_FOOTER + 16.0 + 8.0;
/// Horizontal space a tile takes around the screen (padding + phone frame).
const TILE_SIDES: f32 = TILE_PAD * 2.0 + 8.0;
const GAP: f32 = 12.0;
/// Phone-shaped screen box used before the first frame arrives.
const DEFAULT_ASPECT: f32 = 2400.0 / 1080.0;

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
                        (tile_w - TILE_SIDES) * aspect.clamp(0.4, 2.4)
                    })
                    .fold(0.0, f32::max);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = GAP;
                    for d in row {
                        if let Some(a) =
                            self.tile(ui, d, Vec2::new(tile_w, screen_h + TILE_CHROME), ppp)
                        {
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
        let p = widgets::pal(ui);
        let painter = ui.painter_at(rect.expand(3.0));
        if resp.hovered() {
            painter.rect_stroke(
                rect.expand(1.5),
                RADIUS_CARD + 2,
                Stroke::new(3.0, p.accent_soft),
                StrokeKind::Outside,
            );
        }
        let (edge, width) = if resp.hovered() {
            (p.accent, 1.5)
        } else {
            (p.border, 1.0)
        };
        painter.rect(
            rect,
            RADIUS_CARD,
            p.surface,
            Stroke::new(width, edge),
            StrokeKind::Inside,
        );
        let inner = rect.shrink(TILE_PAD);

        // Header: name and kind on the left, status pill on the right.
        let control = self.capture.control(&serial);
        let (label, tone) = status_badge(d, control.map(|c| c.live.load(Ordering::Relaxed)));
        let pill = widgets::pill_size(&painter, &label);
        let pill_rect = Rect::from_min_size(Pos2::new(inner.right() - pill.x, inner.top()), pill);
        widgets::paint_pill(&painter, p, pill_rect, &label, tone);
        let name = widgets::truncated(
            &painter,
            self.label(d),
            FontId::proportional(13.0),
            p.text,
            (inner.width() - pill.x - 8.0).max(20.0),
        );
        painter.galley(inner.left_top(), name, p.text);
        let kind = widgets::truncated(
            &painter,
            device_kind(d),
            FontId::proportional(11.0),
            p.text3,
            inner.width(),
        );
        painter.galley(inner.left_top() + Vec2::new(0.0, 18.0), kind, p.text3);

        // Screen in a phone frame.
        let frame = Rect::from_min_max(
            Pos2::new(inner.left(), inner.top() + TILE_HEADER + 8.0),
            Pos2::new(inner.right(), inner.bottom() - TILE_FOOTER - 8.0),
        );
        let (screen_box, radius) = widgets::bezel(&painter, p, frame, 18.0);
        if let Some(c) = control {
            c.size.store(
                grid::bucket(screen_box.height().max(screen_box.width()) * ppp),
                Ordering::Relaxed,
            );
        }
        let img = self.paint_screen(&painter, &serial, screen_box, radius);
        if img.is_none() {
            egui::Spinner::new().size(22.0).color(p.text3).paint_at(
                ui,
                Rect::from_center_size(screen_box.center(), Vec2::splat(22.0)),
            );
        }

        // Footer: serial on the left, how the screen arrives on the right.
        let mid = inner.bottom() - TILE_FOOTER / 2.0;
        let transport = match (d.platform, control.map(|c| c.live.load(Ordering::Relaxed))) {
            (Platform::Ios, _) => "simctl",
            (_, Some(true)) => "gRPC",
            _ => "adb · PNG",
        };
        let tag = widgets::tag_size(&painter, transport);
        widgets::paint_tag(
            &painter,
            p,
            Rect::from_min_size(Pos2::new(inner.right() - tag.x, mid - tag.y / 2.0), tag),
            transport,
        );
        let id = widgets::truncated(
            &painter,
            serial.clone(),
            FontId::monospace(11.0),
            p.text3,
            (inner.width() - tag.x - 8.0).max(20.0),
        );
        painter.galley(
            Pos2::new(inner.left(), mid - id.size().y / 2.0),
            id,
            p.text3,
        );

        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("Click to open")
            .clicked()
            .then_some(Action::Focus(serial))
    }

    /// Where the latest frame lands when fitted into `area`.
    fn screen_rect(&self, serial: &str, area: Rect) -> Option<Rect> {
        let screen = self.screens.get(serial)?;
        let [w, h] = screen.size.map(|v| v as f32);
        let scale = (area.width() / w).min(area.height() / h);
        Some(Rect::from_center_size(
            area.center(),
            Vec2::new(w, h) * scale,
        ))
    }

    /// Draw the latest frame fitted into `area` with rounded corners.
    /// Returns where it was drawn.
    fn paint_screen(
        &self,
        painter: &egui::Painter,
        serial: &str,
        area: Rect,
        radius: f32,
    ) -> Option<Rect> {
        let rect = self.screen_rect(serial, area)?;
        let screen = self.screens.get(serial)?;
        widgets::paint_rounded_texture(painter, screen.tex.id(), rect, radius);
        Some(rect)
    }

    fn empty_grid(&self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let p = widgets::pal(ui);
        ui.add_space(32.0);
        ui.vertical_centered(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(132.0, 78.0), Sense::hover());
            let painter = ui.painter();
            for (i, (x, y, w, h)) in [
                (8.0, 14.0, 30.0, 58.0),
                (51.0, 4.0, 30.0, 64.0),
                (94.0, 14.0, 30.0, 58.0),
            ]
            .into_iter()
            .enumerate()
            {
                let color = if i == 1 { p.accent } else { p.border_strong };
                painter.rect_stroke(
                    Rect::from_min_size(rect.min + Vec2::new(x, y), Vec2::new(w, h)),
                    7,
                    Stroke::new(2.0, color),
                    StrokeKind::Inside,
                );
            }
            icons::paint(
                painter,
                Rect::from_center_size(rect.min + Vec2::new(66.0, 36.0), Vec2::splat(16.0)),
                Icon::Play,
                p.accent,
            );
            ui.add_space(8.0);
            ui.heading("No devices running");
            ui.label(
                RichText::new("Start an emulator and its screen shows up here.").color(p.text2),
            );
            ui.add_space(12.0);

            let avds: Vec<&Device> = self
                .devices
                .iter()
                .filter(|d| d.avd.is_some())
                .take(8)
                .collect();
            let width = ui.available_width().min(560.0);
            ui.allocate_ui_with_layout(
                Vec2::new(width, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.columns(2, |cols| {
                        for (i, d) in avds.iter().enumerate() {
                            let ui = &mut cols[i % 2];
                            egui::Frame::new()
                                .fill(p.surface)
                                .stroke(Stroke::new(1.0, p.border))
                                .corner_radius(10)
                                .inner_margin(10)
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.horizontal(|ui| {
                                        widgets::icon(ui, Icon::Phone, 18.0, p.text3);
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if widgets::button(
                                                    ui,
                                                    Kind::Secondary,
                                                    Some(Icon::Play),
                                                    "Start",
                                                )
                                                .clicked()
                                                {
                                                    actions.push(Action::Start(
                                                        d.avd.clone().unwrap_or_default(),
                                                        Boot::Normal,
                                                    ));
                                                }
                                                ui.add(
                                                    egui::Label::new(
                                                        RichText::new(&d.name).strong(),
                                                    )
                                                    .truncate(),
                                                );
                                            },
                                        );
                                    });
                                });
                        }
                    });
                },
            );
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

        let p = widgets::pal(ui);
        let live = self
            .capture
            .control(serial)
            .map(|c| c.live.load(Ordering::Relaxed));
        let (badge, tone) = status_badge(&d, live);
        let others: Vec<(String, String)> = self
            .running()
            .filter_map(|o| Some((o.serial()?.to_string(), self.label(o))))
            .collect();
        let current = self.label(&d);

        let compact = ui.available_width() < 850.0;
        let switch_width = if compact {
            (ui.available_width() - 185.0).clamp(80.0, 220.0)
        } else {
            220.0
        };
        ui.horizontal(|ui| {
            if widgets::button(ui, Kind::Ghost, Some(Icon::ChevronLeft), "Grid")
                .on_hover_text("Esc")
                .clicked()
            {
                actions.push(Action::Unfocus);
            }
            egui::ComboBox::from_id_salt("device-switch")
                .selected_text(RichText::new(current).strong())
                .width(switch_width)
                .show_ui(ui, |ui| {
                    for (s, name) in others {
                        if ui.selectable_label(s == serial, name).clicked() && s != serial {
                            actions.push(Action::Focus(s));
                        }
                    }
                })
                .response
                .on_hover_text("Switch device");
            widgets::status_pill(ui, &badge, tone);
            if !compact {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    focused_toolbar_controls(ui, &d, serial, actions);
                });
            }
        });
        if compact {
            ui.horizontal_wrapped(|ui| focused_toolbar_controls(ui, &d, serial, actions));
        }
        ui.add_space(4.0);

        let hint_h = if android && ui.available_width() < 700.0 {
            76.0
        } else {
            28.0
        };
        let rail_w = if android { 64.0 } else { 0.0 };
        let area = ui.available_rect_before_wrap();
        let area = Rect::from_min_max(
            area.min,
            Pos2::new(area.right(), (area.bottom() - hint_h).max(area.top())),
        );
        ui.painter()
            .rect_filled(area, 12, p.bg.lerp_to_gamma(Color32::BLACK, 0.10));
        let phone_area = Rect::from_min_max(area.min, area.max - Vec2::new(rail_w, 0.0));
        let (rect, resp) = ui.allocate_exact_size(phone_area.size(), Sense::click_and_drag());
        let ppp = ui.ctx().pixels_per_point();
        if let Some(c) = self.capture.control(serial) {
            c.size.store(
                grid::bucket(rect.height().max(rect.width()) * ppp),
                Ordering::Relaxed,
            );
        }
        self.visible.insert(serial.to_string());
        let painter = ui.painter_at(rect);
        let frame_radius = 34.0;
        let pad = widgets::bezel_pad(frame_radius);
        let inset = (pad + 6.0).min((rect.width().min(rect.height()) / 2.0).max(0.0));
        let screen_area = rect.shrink(inset);
        let fit = self.screen_rect(serial, screen_area).unwrap_or_else(|| {
            let height = screen_area
                .height()
                .max(0.0)
                .min(screen_area.width().max(0.0) * DEFAULT_ASPECT);
            Rect::from_center_size(
                screen_area.center(),
                Vec2::new(height / DEFAULT_ASPECT, height),
            )
        });
        let frame = fit.expand(pad);
        let (inner, radius) = widgets::bezel(&painter, p, frame, frame_radius);
        let img = self.paint_screen(&painter, serial, inner, radius);
        if img.is_none() {
            egui::Spinner::new().size(24.0).color(p.text3).paint_at(
                ui,
                Rect::from_center_size(inner.center(), Vec2::splat(24.0)),
            );
        }

        // Control rail next to the phone (Android).
        if android {
            let items = if d.is_emulator { 9.0 } else { 8.0 };
            let height = (items * 44.0 + 3.0 * 9.0 + 12.0_f32).min(area.height().max(0.0));
            let top = (area.center().y - height / 2.0).max(area.top());
            let rail_rect = Rect::from_min_size(
                Pos2::new(
                    (frame.right() + 20.0).min(area.right() - (rail_w - 8.0)),
                    top,
                ),
                Vec2::new(rail_w - 8.0, height),
            );
            let mut rail = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rail_rect)
                    .layout(egui::Layout::top_down(egui::Align::Center)),
            );
            egui::Frame::new()
                .fill(p.surface)
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(RADIUS_CARD)
                .inner_margin(6)
                .show(&mut rail, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("focused-control-rail")
                        .max_height((height - 12.0).max(0.0))
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 4.0;
                            let sep = |ui: &mut egui::Ui| {
                                let (r, _) =
                                    ui.allocate_exact_size(Vec2::new(28.0, 9.0), Sense::hover());
                                ui.painter().hline(
                                    r.x_range(),
                                    r.center().y,
                                    Stroke::new(1.0, p.border),
                                );
                            };
                            let groups: [&[(Icon, &str, u32)]; 3] = [
                                &[
                                    (Icon::Back, "Back", 4),
                                    (Icon::Home, "Home", 3),
                                    (Icon::Recents, "Recent apps", 187),
                                ],
                                &[
                                    (Icon::VolumeDown, "Volume down", 25),
                                    (Icon::VolumeUp, "Volume up", 24),
                                ],
                                &[(Icon::Power, "Power", 26)],
                            ];
                            for (i, group) in groups.iter().enumerate() {
                                if i > 0 {
                                    sep(ui);
                                }
                                for &(icon, tip, code) in group.iter() {
                                    if widgets::icon_button(ui, icon, tip, 40.0, false).clicked() {
                                        actions.push(shell(&format!("input keyevent {code}")));
                                    }
                                }
                                if i == 2
                                    && d.is_emulator
                                    && widgets::icon_button(
                                        ui,
                                        Icon::Rotate,
                                        "Rotate the emulator",
                                        40.0,
                                        false,
                                    )
                                    .clicked()
                                {
                                    actions.push(Action::Rotate(serial.to_string()));
                                }
                            }
                            sep(ui);
                            if widgets::icon_button(
                                ui,
                                Icon::Camera,
                                "Save screenshot to Desktop",
                                40.0,
                                false,
                            )
                            .clicked()
                            {
                                actions.push(Action::Screenshot(serial.to_string()));
                            }
                        });
                });
        }

        ui.vertical_centered(|ui| {
            if android {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for (key, description) in [
                        ("Click", "tap"),
                        ("Drag", "swipe"),
                        ("Hold", "long press, then drag to move"),
                        ("Type", "send keys"),
                        ("Esc", "back"),
                    ] {
                        widgets::kbd(ui, key);
                        ui.label(RichText::new(description).size(11.0).color(p.text3));
                        ui.add_space(4.0);
                    }
                });
            } else {
                ui.label(
                    RichText::new("View only — use “Open in Simulator” to interact")
                        .size(11.0)
                        .color(p.text3),
                );
            }
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
        let p = widgets::pal(ui);
        egui::ScrollArea::vertical().show(ui, |ui| {
            let of = |platform: Platform, emulator: bool| -> Vec<&Device> {
                self.devices
                    .iter()
                    .filter(|d| d.platform == platform && d.is_emulator == emulator)
                    .collect()
            };
            let emulators = of(Platform::Android, true);
            let physical = of(Platform::Android, false);
            if emulators.is_empty() && physical.is_empty() && !self.scanning {
                ui.label(RichText::new("No Android emulators or devices found.").color(p.text2));
            }
            device_group(ui, "Android emulators", &emulators, actions);
            device_group(ui, "Physical devices", &physical, actions);

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
                    device_rows(ui, &ios, actions);
                });
            } else if cfg!(target_os = "macos") && !self.sdk.simctl {
                ui.add_space(8.0);
                ui.label(
                    RichText::new("iOS simulators: install Xcode to see and control them here.")
                        .size(11.0)
                        .color(p.text3),
                );
            }
        });
    }
}

fn focused_toolbar_controls(
    ui: &mut egui::Ui,
    d: &Device,
    serial: &str,
    actions: &mut Vec<Action>,
) {
    let p = widgets::pal(ui);
    let android = d.platform == Platform::Android;
    if d.is_emulator && widgets::button(ui, Kind::Danger, Some(Icon::Stop), "Stop").clicked() {
        actions.push(match &d.udid {
            Some(udid) => Action::IosShutdown(udid.clone()),
            None => Action::Stop(serial.to_string()),
        });
    }
    if widgets::icon_button(ui, Icon::Camera, "Save screenshot to Desktop", 32.0, true).clicked() {
        actions.push(Action::Screenshot(serial.to_string()));
    }
    if android {
        egui::Frame::new()
            .fill(p.accent_soft)
            .stroke(Stroke::new(1.0, p.accent.gamma_multiply(0.5)))
            .corner_radius(255)
            .inner_margin(egui::Margin::symmetric(12, 4))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    widgets::icon(ui, Icon::Keyboard, 15.0, p.accent_text);
                    ui.label(
                        RichText::new("Typing goes to the device")
                            .size(12.0)
                            .color(p.accent_text),
                    );
                    widgets::kbd(ui, "Esc");
                    ui.label(RichText::new("to leave").size(12.0).color(p.accent_text));
                });
            });
    } else {
        if let Some(udid) = &d.udid
            && widgets::button(ui, Kind::Secondary, None, "Open in Simulator").clicked()
        {
            actions.push(Action::IosOpen(udid.clone()));
        }
    }
}

/// What a device is, in a few words.
fn device_kind(d: &Device) -> String {
    match (d.platform, d.is_emulator) {
        (Platform::Ios, _) => format!("iOS simulator · {}", d.runtime.as_deref().unwrap_or("iOS")),
        (_, true) => "Android emulator".into(),
        (_, false) => "USB device".into(),
    }
}

/// Short status for a running device: how its screen is being captured.
fn status_badge(d: &Device, live: Option<bool>) -> (String, Tone) {
    match (&d.state, live) {
        (State::Running { .. }, Some(true)) => ("LIVE".into(), Tone::Live),
        (State::Running { .. }, Some(false)) => ("2 FPS".into(), Tone::Polled),
        (State::Running { .. }, None) => ("STARTING".into(), Tone::Busy),
        (State::Other { status, .. }, _) => (status.to_uppercase(), Tone::Busy),
        (State::Stopped, _) => ("STOPPED".into(), Tone::Stopped),
    }
}

/// A titled, rounded group of device rows.
fn device_group(ui: &mut egui::Ui, title: &str, rows: &[&Device], actions: &mut Vec<Action>) {
    if rows.is_empty() {
        return;
    }
    ui.add_space(6.0);
    widgets::section_label(ui, &format!("{title} · {}", rows.len()));
    device_rows(ui, rows, actions);
}

// Shared column rectangles keep headings and values aligned at every width.
fn device_columns(rect: Rect) -> [Rect; 4] {
    let width = rect.width();
    let actions = 175.0_f32.min(width * 0.48);
    let state = 105.0_f32.min((width - actions) * 0.55);
    let serial = if width > 600.0 {
        160.0_f32.min((width - actions - state) * 0.4)
    } else {
        0.0
    };
    let x = [
        rect.left(),
        rect.right() - actions - serial - state,
        rect.right() - actions - serial,
        rect.right() - actions,
        rect.right(),
    ];
    std::array::from_fn(|i| {
        Rect::from_min_max(
            Pos2::new(x[i], rect.top()),
            Pos2::new(x[i + 1], rect.bottom()),
        )
    })
}

fn device_rows(ui: &mut egui::Ui, rows: &[&Device], actions: &mut Vec<Action>) {
    let p = widgets::pal(ui);
    egui::Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(12)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            let (header, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), Sense::hover());
            let columns = device_columns(header);
            for (rect, title) in columns
                .into_iter()
                .zip(["Device", "State", "Serial", "Actions"])
            {
                if rect.width() > 0.0 {
                    let mut cell = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(rect.shrink2(Vec2::new(8.0, 0.0)))
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    );
                    widgets::section_label(&mut cell, title);
                }
            }
            for d in rows {
                let (r, _) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
                ui.painter().rect_filled(r, 0, p.border);
                device_row(ui, d, actions);
            }
        });
}

fn device_row(ui: &mut egui::Ui, d: &Device, actions: &mut Vec<Action>) {
    let p = widgets::pal(ui);
    let (pill, tone) = match &d.state {
        State::Running { .. } if d.platform == Platform::Ios => ("BOOTED".to_string(), Tone::Live),
        State::Running { .. } => ("RUNNING".to_string(), Tone::Live),
        State::Other { status, .. } => (status.to_uppercase(), Tone::Busy),
        State::Stopped => ("STOPPED".to_string(), Tone::Stopped),
    };
    let (dot, _) = tone.colors(p);
    let serial = match &d.state {
        State::Running { serial } | State::Other { serial, .. } => serial.as_str(),
        State::Stopped => "—",
    };
    let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 58.0), Sense::hover());
    let columns = device_columns(row);
    let mut device = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(columns[0].shrink2(Vec2::new(8.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let (r, _) = device.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
    widgets::paint_dot(device.painter(), r.center(), dot, tone == Tone::Stopped);
    device.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.add(egui::Label::new(RichText::new(&d.name).strong()).truncate());
        ui.add(
            egui::Label::new(RichText::new(device_kind(d)).size(11.0).color(p.text3)).truncate(),
        );
    });
    let mut state = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(columns[1].shrink2(Vec2::new(8.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    widgets::status_pill(&mut state, &pill, tone);
    if columns[2].width() > 0.0 {
        let mut cell = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(columns[2].shrink2(Vec2::new(8.0, 0.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        cell.add(
            egui::Label::new(RichText::new(serial).monospace().size(11.5).color(p.text3))
                .truncate(),
        );
    }
    let mut cell = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(columns[3].shrink2(Vec2::new(8.0, 0.0)))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    let ui = &mut cell;
    match (&d.state, d.platform) {
        (State::Stopped, Platform::Android) => {
            if let Some(avd) = &d.avd {
                let main = widgets::split_button(ui, "Start", |ui| {
                    if widgets::menu_item(
                        ui,
                        "Cold boot",
                        "Ignore the snapshot and boot from scratch",
                        false,
                    )
                    .clicked()
                    {
                        actions.push(Action::Start(avd.clone(), Boot::Cold));
                    }
                    if widgets::menu_item(
                        ui,
                        "Wipe data and start",
                        "Erases every app and file on this AVD",
                        true,
                    )
                    .clicked()
                    {
                        actions.push(Action::Start(avd.clone(), Boot::Wipe));
                    }
                });
                if main.clicked() {
                    actions.push(Action::Start(avd.clone(), Boot::Normal));
                }
            }
        }
        (State::Stopped, Platform::Ios) => {
            if let Some(udid) = &d.udid
                && widgets::button(ui, Kind::Primary, Some(Icon::Play), "Boot").clicked()
            {
                actions.push(Action::IosBoot(udid.clone()));
            }
        }
        (State::Running { serial }, platform) => {
            if d.is_emulator && widgets::button(ui, Kind::Ghost, Some(Icon::Stop), "Stop").clicked()
            {
                actions.push(match (platform, &d.udid) {
                    (Platform::Ios, Some(udid)) => Action::IosShutdown(udid.clone()),
                    _ => Action::Stop(serial.clone()),
                });
            }
            if widgets::button(ui, Kind::Secondary, None, "Open").clicked() {
                actions.push(Action::Focus(serial.clone()));
            }
        }
        _ => {}
    }
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

        let p = theme::palette(ctx);
        egui::Panel::top("top")
            .frame(
                egui::Frame::new()
                    .fill(p.chrome)
                    .stroke(Stroke::new(1.0, p.border))
                    .inner_margin(egui::Margin::symmetric(14, 10)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    widgets::wordmark(ui);
                    ui.add_space(8.0);
                    let selected = usize::from(self.view != View::Grid);
                    let items = [(Icon::Grid, "Grid"), (Icon::List, "List")];
                    match widgets::segmented(ui, &items, selected) {
                        Some(0) => actions.push(Action::View(View::Grid)),
                        Some(_) => actions.push(Action::View(View::List)),
                        None => {}
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::icon_button(ui, Icon::Refresh, "Refresh devices", 32.0, true)
                            .clicked()
                        {
                            actions.push(Action::Refresh);
                        }
                        if self.scanning {
                            ui.spinner();
                        }
                    });
                });
            });

        egui::Panel::bottom("status")
            .frame(
                egui::Frame::new()
                    .fill(p.chrome)
                    .stroke(Stroke::new(1.0, p.border))
                    .inner_margin(egui::Margin::symmetric(14, 6)),
            )
            .show(ui, |ui| {
                let running = self.running().count();
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 14.0;
                    ui.label(
                        RichText::new(format!(
                            "{} devices · {running} running",
                            self.devices.len()
                        ))
                        .size(11.5)
                        .color(p.text3),
                    );
                    if let Some(msg) = &self.message {
                        if msg.starts_with("Error:") {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 5.0;
                                widgets::icon(ui, Icon::Warning, 13.0, p.danger);
                                ui.label(RichText::new(msg).size(11.5).color(p.danger));
                            });
                        } else {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 5.0;
                                let (r, _) =
                                    ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                                ui.painter().line_segment(
                                    [r.left_center(), r.left_bottom() - Vec2::new(-4.0, 2.0)],
                                    Stroke::new(1.5, p.live),
                                );
                                ui.painter().line_segment(
                                    [
                                        r.left_bottom() - Vec2::new(-4.0, 2.0),
                                        r.right_top() + Vec2::new(0.0, 2.0),
                                    ],
                                    Stroke::new(1.5, p.live),
                                );
                                ui.label(RichText::new(msg).size(11.5).color(p.text2));
                            });
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new("Drop an .apk to install it everywhere")
                                .size(11.5)
                                .color(p.text3),
                        )
                        .on_hover_text("Installs on every running Android device");
                    });
                });
            });

        egui::CentralPanel::default_margins()
            .frame(
                egui::Frame::new()
                    .fill(p.bg)
                    .inner_margin(egui::Margin::same(16)),
            )
            .show(ui, |ui| {
                if self.sdk.adb.is_none() && self.sdk.emulator.is_none() && !self.sdk.simctl {
                    ui.add_space(40.0);
                    ui.vertical_centered(|ui| {
                        egui::Frame::new()
                            .fill(p.surface)
                            .stroke(Stroke::new(1.0, p.border))
                            .corner_radius(RADIUS_CARD)
                            .inner_margin(24)
                            .show(ui, |ui| {
                                ui.vertical_centered(|ui| {
                                    let (r, _) =
                                        ui.allocate_exact_size(Vec2::splat(44.0), Sense::hover());
                                    ui.painter().rect_filled(r, 12, p.polled_soft);
                                    icons::paint(
                                        ui.painter(),
                                        Rect::from_center_size(r.center(), Vec2::splat(22.0)),
                                        Icon::Warning,
                                        p.polled,
                                    );
                                    ui.add_space(6.0);
                                    ui.heading("Android SDK not found");
                                    ui.label(
                            RichText::new("Set ANDROID_HOME or put adb and emulator on your PATH.")
                                .color(p.text2),
                        );
                                });
                            });
                    });
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
            let panel = Rect::from_center_size(
                full.center(),
                Vec2::new(
                    (full.width() - 32.0).clamp(0.0, 440.0),
                    160.0_f32.min(full.height()),
                ),
            );
            painter.rect_filled(panel, 16, p.popup);
            let c = panel.shrink(10.0);
            let path = [
                c.left_top(),
                c.right_top(),
                c.right_bottom(),
                c.left_bottom(),
                c.left_top(),
            ];
            painter.extend(egui::Shape::dashed_line(
                &path,
                Stroke::new(1.5, p.accent),
                7.0,
                5.0,
            ));
            icons::paint(
                &painter,
                Rect::from_center_size(panel.center() - Vec2::new(0.0, 26.0), Vec2::splat(32.0)),
                Icon::Upload,
                p.accent,
            );
            let text = painter.layout(
                "Drop .apk to install on all running Android devices".to_owned(),
                FontId::proportional(14.0),
                p.text,
                (panel.width() - 40.0).max(1.0),
            );
            painter.galley(
                panel.center() + Vec2::new(-text.size().x / 2.0, 20.0),
                text,
                p.text,
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
        let sources = [include_str!("app.rs"), include_str!("widgets.rs")];
        let mut chars: Vec<char> = sources
            .iter()
            .flat_map(|s| s.chars())
            .filter(|c| !c.is_ascii())
            .collect();
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

    #[test]
    fn list_columns_hide_serial_on_narrow_windows() {
        for width in [328.0, 600.0, 800.0] {
            let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 58.0));
            let cols = device_columns(rect);
            assert_eq!(cols[2].width() > 0.0, width > 600.0);
            assert_eq!(cols[0].left(), rect.left());
            assert_eq!(cols[3].right(), rect.right());
            for pair in cols.windows(2) {
                assert_eq!(pair[0].right(), pair[1].left());
            }
        }
    }

    #[test]
    fn focused_device_without_frame_still_draws_a_bezel() {
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
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 700.0))),
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| {
            app.focused_view(ui, "emulator-5554", &mut Vec::new());
        });
        out.textures_delta.clear();
        assert!(out.shapes.iter().any(|shape| matches!(
            &shape.shape,
            egui::Shape::Rect(rect) if rect.fill == crate::theme::BEZEL
        )));
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
