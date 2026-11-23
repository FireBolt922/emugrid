//! The egui window: a grid of live screens, a focused (interactive) device
//! view, and the device list with start / stop controls.

use crate::capture::{Manager, Source, grpc};
use crate::devices::{self, Boot, Device, Platform, State};
use crate::icons::{self, Icon};
use crate::sdk::Sdk;
use crate::settings::Settings;
use crate::shortcuts::{self, Command as KeyCommand};
use crate::theme::{self, RADIUS_CARD, Tone};
use crate::widgets::{self, Kind};
use crate::{grid, input};
use eframe::egui::{self, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

#[path = "ui/focused.rs"]
mod focused;
use focused::Zoom;
#[path = "ui/grid.rs"]
mod grid_view;
#[path = "ui/list.rs"]
mod list;
use list::DeviceTab;
#[path = "ui/setup.rs"]
mod setup;
#[path = "ui/sidebar.rs"]
mod sidebar;
#[path = "ui/toasts.rs"]
mod toasts;
use toasts::{Notice, Toasts};
#[path = "ui/toolbar.rs"]
mod toolbar;
use toolbar::Dialog;

const REFRESH_EVERY: Duration = Duration::from_secs(3);
const BOOT_DISCOVERY_GRACE: Duration = Duration::from_secs(120);
const SCROLL_INTERVAL: Duration = Duration::from_millis(100);
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
    Scan(u64, Vec<Device>, HashMap<String, grpc::Endpoint>),
    Status(Result<String, String>),
    InputError(String),
    InstallDone(u64, String, Result<(), devices::InstallFailure>),
    /// `None` when `wm size` failed; it is asked again.
    ScreenSize(String, u64, Option<(u32, u32)>),
    #[cfg(target_os = "linux")]
    SystemTheme(Option<egui::Theme>),
}

/// Things the user asked for while the UI was being drawn.
enum Action {
    Focus(String),
    Unfocus,
    View(View),
    Refresh,
    RecheckSdk,
    SaveSettings,
    ToggleSidebar,
    Install(PathBuf),
    RetryInstall(u64, String),
    Dialog(Dialog),
    Zoom(Zoom),
    UseSdk(PathBuf),
    CopiedCommand,
    Start(String, Boot),
    Stop(String),
    StopLaunch(String),
    IosBoot(String),
    IosShutdown(String),
    IosOpen(String),
    Rotate(String),
    Screenshot(String),
    /// `adb shell <cmd>` on a serial, run in order with other input.
    Shell(String, String),
    Scroll(String, input::adb::Command),
}

struct InstallJob {
    id: u64,
    apk: PathBuf,
    downgrade: bool,
}

struct Screen {
    tex: egui::TextureHandle,
    first_pixel: Color32,
    size: [usize; 2],
}

impl Screen {
    fn aspect(&self) -> f32 {
        self.size[1] as f32 / self.size[0].max(1) as f32
    }
}

pub struct EmuGrid {
    starts: HashMap<String, (Instant, Boot)>,
    launches: HashMap<String, Child>,
    sdk: Sdk,
    settings: Settings,
    sdk_checks: Vec<crate::sdk::Check>,
    sdk_path: Option<String>,
    sdk_generation: u64,
    devices: Vec<Device>,
    endpoints: HashMap<String, grpc::Endpoint>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    scanning: bool,
    scanned: bool,
    #[cfg(target_os = "linux")]
    theme_check: Option<Instant>,
    last_scan: Option<Instant>,
    message: Option<String>,
    toasts: Toasts,
    key_commands: Vec<KeyCommand>,
    editing_text: bool,
    popup_has_keys: bool,
    focus_filter: bool,
    filter: String,
    tile_size_dirty: bool,
    grid_platform: Option<Platform>,
    dialog: Option<Dialog>,
    apk_path: String,
    device_tab: DeviceTab,
    install_queue: HashMap<String, VecDeque<InstallJob>>,
    details_open: bool,
    sidebar_overlay: bool,
    send_text: String,
    zoom: Zoom,
    stage_dots: Option<egui::TextureHandle>,
    ripple: Option<(Vec2, Instant)>,
    view: View,
    /// Serial of the device shown large.
    focused: Option<String>,
    capture: Manager,
    screens: HashMap<String, Screen>,
    fps: HashMap<String, (u64, Option<u32>)>,
    fps_at: Instant,
    /// Real screen size per Android serial (`wm size`), for mapping taps.
    screen_sizes: HashMap<String, (u32, u32)>,
    input_workers: HashMap<String, input::adb::Worker>,
    /// Tiles drawn on screen this frame; everything else pauses.
    visible: HashSet<String>,
    last_grid_visible: HashSet<String>,
    /// Accumulated scroll (points) and when the last swipe was sent.
    scroll: (f32, Option<Instant>),
    gestures: input::GestureTracker,
    /// Tab presses taken from egui, to send to the focused device.
    pending_tabs: usize,
    /// A live finger held down on a device: serial and last device position.
    finger: Option<(String, (u32, u32))>,
    /// Serials with a `wm size` lookup in flight.
    size_pending: HashMap<String, u64>,
    next_size_request: u64,
}

impl EmuGrid {
    pub fn new(ctx: &egui::Context) -> Self {
        let settings = Settings::load();
        let sdk = Sdk::detect(settings.sdk_root.as_deref());
        let sdk_checks = crate::sdk::diagnose(settings.sdk_root.as_deref());
        let (tx, rx) = mpsc::channel();

        Self {
            starts: HashMap::new(),
            launches: HashMap::new(),
            sdk,
            settings,
            sdk_checks,
            sdk_path: None,
            sdk_generation: 0,
            devices: Vec::new(),
            endpoints: HashMap::new(),
            tx,
            rx,
            scanning: false,
            scanned: false,
            #[cfg(target_os = "linux")]
            theme_check: None,
            last_scan: None,
            message: None,
            toasts: Toasts::default(),
            key_commands: Vec::new(),
            editing_text: false,
            popup_has_keys: false,
            focus_filter: false,
            filter: String::new(),
            tile_size_dirty: false,
            grid_platform: None,
            dialog: None,
            apk_path: String::new(),
            device_tab: DeviceTab::All,
            install_queue: HashMap::new(),
            details_open: false,
            sidebar_overlay: false,
            send_text: String::new(),
            zoom: Zoom::Fit,
            stage_dots: None,
            ripple: None,
            view: View::Grid,
            focused: None,
            capture: Manager::new(ctx.clone()),
            screens: HashMap::new(),
            fps: HashMap::new(),
            fps_at: Instant::now(),
            screen_sizes: HashMap::new(),
            input_workers: HashMap::new(),
            visible: HashSet::new(),
            last_grid_visible: HashSet::new(),
            scroll: (0.0, None),
            gestures: input::GestureTracker::default(),
            pending_tabs: 0,
            size_pending: HashMap::new(),
            next_size_request: 0,
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
        let generation = self.sdk_generation;
        thread::spawn(move || {
            let _ = tx.send(Msg::Scan(generation, devices::scan(&sdk), grpc::discover()));
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
            let _ = tx.send(Msg::Status(job(&sdk)));
            ctx.request_repaint();
        });
    }

    fn on_scan(&mut self, list: Vec<Device>, endpoints: HashMap<String, grpc::Endpoint>) {
        self.devices = list;
        for d in &mut self.devices {
            if let Some(avd) = &d.avd {
                match d.state {
                    State::Running { .. } => {
                        self.starts.remove(avd);
                    }
                    State::Stopped if self.starts.contains_key(avd) => {
                        d.state = State::Booting { serial: None };
                    }
                    State::Other {
                        ref serial,
                        ref status,
                    } if status == "offline" && self.starts.contains_key(avd) => {
                        d.state = State::Booting {
                            serial: Some(serial.clone()),
                        };
                    }
                    _ => {}
                }
            }
        }
        self.endpoints = endpoints;
        self.scanning = false;
        self.scanned = true;

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
        self.fps.retain(|s, _| alive.contains(s));
        self.screen_sizes.retain(|s, _| alive.contains(s));
        self.size_pending.retain(|s, _| alive.contains(s));
        if self.focused.as_ref().is_some_and(|f| !alive.contains(f)) {
            self.release_focus();
        }
        self.input_workers
            .retain(|serial, _| alive.contains(serial));
    }

    /// Look up an Android device's real screen size (for mapping taps) until
    /// it is known. Failed lookups are retried after a pause.
    fn ensure_screen_size(&mut self, ctx: &egui::Context, serial: &str) {
        if self.screen_sizes.contains_key(serial) || self.size_pending.contains_key(serial) {
            return;
        }
        self.next_size_request += 1;
        let ticket = self.next_size_request;
        self.size_pending.insert(serial.into(), ticket);
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
            let _ = tx.send(Msg::ScreenSize(s, ticket, size));
            ctx.request_repaint();
        });
    }

    /// Never leave a finger pressed on a device we stop looking at.
    fn lift_finger(&mut self) {
        self.gestures = input::GestureTracker::default();
        self.scroll = (0.0, None);
        if let Some((serial, (x, y))) = self.finger.take() {
            self.send_input(
                &serial,
                input::adb::Command::Shell(format!("input motionevent CANCEL {x} {y}")),
            );
        }
    }

    fn prepare_input(&mut self, ctx: &egui::Context, serial: &str) {
        if let Some(adb) = self.sdk.adb.clone() {
            self.input_workers.entry(serial.into()).or_insert_with(|| {
                let (tx, ctx) = (self.tx.clone(), ctx.clone());
                input::adb::Worker::start(adb, serial.into(), move |message| {
                    let _ = tx.send(Msg::InputError(message));
                    ctx.request_repaint();
                })
            });
        }
    }

    fn send_input(&self, serial: &str, command: input::adb::Command) {
        if let Some(worker) = self.input_workers.get(serial) {
            worker.send(command, self.capture.control(serial).cloned());
        }
    }

    /// Name shown for a device; emulators often share a model name, so add
    /// the serial when the name alone is ambiguous.
    fn label(&self, d: &Device) -> String {
        let same = self
            .running()
            .filter(|o| o.display_name == d.display_name)
            .count();
        match d.serial() {
            Some(serial) if same > 1 && d.platform == Platform::Android => {
                format!("{} · {serial}", d.title())
            }
            _ => d.title(),
        }
    }

    fn running(&self) -> impl Iterator<Item = &Device> {
        self.devices.iter().filter(|d| d.serial().is_some())
    }

    fn device(&self, serial: &str) -> Option<&Device> {
        self.devices.iter().find(|d| d.serial() == Some(serial))
    }

    fn save_settings(&mut self) {
        if let Err(error) = self.settings.save() {
            self.toasts
                .push(Notice::Error, format!("Could not save settings: {error}"));
        }
    }

    fn recheck_sdk(&mut self) {
        let sdk = Sdk::detect(self.settings.sdk_root.as_deref());
        if self.sdk.adb != sdk.adb {
            self.release_focus();
            self.capture.sync(Vec::new());
            self.devices.clear();
            self.screens.clear();
            self.fps.clear();
            self.input_workers.clear();
            self.screen_sizes.clear();
            self.size_pending.clear();
        }
        self.sdk = sdk;
        self.scanned = false;
        self.sdk_checks = crate::sdk::diagnose(self.settings.sdk_root.as_deref());
        self.sdk_generation += 1;
        self.scanning = false;
        self.last_scan = None;
    }

    fn device_shortcut(&self, serial: &str, command: KeyCommand) -> Option<Action> {
        let device = self.device(serial)?;
        let code = match command {
            KeyCommand::Filter => return None,
            KeyCommand::Device(index) => {
                return self
                    .running()
                    .nth(index)?
                    .serial()
                    .map(|s| Action::Focus(s.into()));
            }
            KeyCommand::Screenshot => return Some(Action::Screenshot(serial.into())),
            _ if device.platform != Platform::Android => return None,
            KeyCommand::Rotate => return device.is_emulator.then(|| Action::Rotate(serial.into())),
            KeyCommand::Back => 4,
            KeyCommand::Home => 3,
            KeyCommand::Recents => 187,
            KeyCommand::VolumeDown => 25,
            KeyCommand::VolumeUp => 24,
            KeyCommand::Power => 26,
        };
        Some(Action::Shell(
            serial.into(),
            format!("input keyevent {code}"),
        ))
    }

    fn set_zoom(&mut self, zoom: Zoom) {
        if self.zoom == zoom {
            return;
        }
        self.lift_finger();
        self.zoom = zoom;
        if let Some(screen) = self
            .focused
            .as_deref()
            .and_then(|serial| self.screens.get_mut(serial))
        {
            // Texture deltas also update sampling, even before the next frame arrives.
            screen.tex.set_partial(
                [0, 0],
                egui::ColorImage::filled([1, 1], screen.first_pixel),
                zoom.texture_options(),
            );
        }
    }

    fn release_focus(&mut self) {
        self.set_zoom(Zoom::Fit);
        self.lift_finger();
        self.focused = None;
        self.send_text.clear();
        self.details_open = false;
        self.ripple = None;
    }

    fn apply(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::Focus(serial) => {
                if self.focused.as_deref() == Some(&serial) || self.device(&serial).is_none() {
                    return;
                }
                self.release_focus();
                if self
                    .device(&serial)
                    .is_some_and(|d| d.platform == Platform::Android)
                {
                    self.prepare_input(ctx, &serial);
                }
                self.focused = Some(serial);
                self.view = View::Grid;
            }
            Action::Unfocus => self.release_focus(),
            Action::View(v) => {
                self.release_focus();
                self.view = v;
            }
            Action::Zoom(zoom) => self.set_zoom(zoom),
            Action::Refresh => self.last_scan = None,
            Action::RecheckSdk => self.recheck_sdk(),
            Action::SaveSettings => self.save_settings(),
            Action::ToggleSidebar => {
                if self.view == View::Grid
                    && self.focused.is_none()
                    && ctx.content_rect().width() < sidebar::SIDEBAR_MIN
                {
                    self.sidebar_overlay = !self.sidebar_overlay;
                    self.settings.sidebar = self.sidebar_overlay;
                } else {
                    self.settings.sidebar = !self.settings.sidebar;
                }
                self.save_settings();
            }
            Action::Install(path) => self.install_dropped(ctx, vec![path]),
            Action::RetryInstall(id, serial) => {
                if let Some((apk, downgrade)) = self.toasts.retry_install(id, &serial) {
                    self.enqueue_install(ctx, serial, InstallJob { id, apk, downgrade });
                }
            }
            Action::Dialog(dialog) => {
                self.lift_finger();
                if dialog == Dialog::Sdk {
                    self.sdk_path = self
                        .settings
                        .sdk_root
                        .as_ref()
                        .or(self.sdk.root.as_ref())
                        .map(|path| path.to_string_lossy().into_owned())
                        .or_else(|| Some(String::new()));
                }
                self.dialog = Some(dialog);
            }
            Action::UseSdk(path) => {
                let path = crate::sdk::resolve_path(&path);
                match path {
                    Ok(path) if crate::sdk::has_adb(&path) => {
                        self.settings.sdk_root = Some(path);
                        self.sdk_path = None;
                        if self.dialog == Some(Dialog::Sdk) {
                            self.dialog = None;
                        }
                        self.save_settings();
                        self.recheck_sdk();
                        self.toasts
                            .push(Notice::Success, "Android SDK folder selected");
                    }
                    _ => self.toasts.push(
                        Notice::Error,
                        "Choose an SDK folder containing platform-tools/adb",
                    ),
                }
            }
            Action::CopiedCommand => self.toasts.push(Notice::Info, "Copied SDK install command"),
            Action::Start(avd, boot) => {
                let how = match boot {
                    Boot::Normal => "Starting",
                    Boot::Cold => "Cold booting",
                    Boot::Wipe => "Wiping and starting",
                };
                if self.launches.contains_key(&avd)
                    || !self
                        .devices
                        .iter()
                        .any(|d| d.avd.as_deref() == Some(&avd) && d.state == State::Stopped)
                {
                    return;
                }
                match devices::start(&self.sdk, &avd, boot) {
                    Ok(child) => {
                        self.starts.insert(avd.clone(), (Instant::now(), boot));
                        self.launches.insert(avd.clone(), child);
                        if let Some(d) = self
                            .devices
                            .iter_mut()
                            .find(|d| d.avd.as_deref() == Some(&avd))
                        {
                            d.state = State::Booting { serial: None };
                        }
                        self.message = Some(format!("{how} {avd}…"));
                    }
                    Err(e) => self.toasts.push(Notice::Error, e),
                }
                self.last_scan = None;
            }
            Action::StopLaunch(avd) => {
                match self.launches.get_mut(&avd) {
                    Some(child) => match child.kill() {
                        Ok(()) => {
                            self.starts.remove(&avd);
                        }
                        Err(e) => self.toasts.push(Notice::Error, e.to_string()),
                    },
                    None => self.toasts.push(
                        Notice::Info,
                        "Waiting for adb to report the emulator before it can be stopped",
                    ),
                }
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
                    self.toasts.push(Notice::Error, e);
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
                if self
                    .device(&serial)
                    .is_some_and(|d| d.platform == Platform::Android)
                {
                    self.prepare_input(ctx, &serial);
                }
                self.send_input(&serial, input::adb::Command::Shell(cmd));
            }
            Action::Scroll(serial, command) => self.send_input(&serial, command),
        }
    }

    /// Install dropped `.apk` files on every running Android device.
    fn install_dropped(&mut self, ctx: &egui::Context, apks: Vec<PathBuf>) {
        let targets: Vec<_> = self
            .running()
            .filter(|d| d.platform == Platform::Android)
            .filter_map(|d| Some((d.serial()?.to_owned(), self.label(d))))
            .collect();
        if targets.is_empty() {
            self.toasts
                .push(Notice::Info, "No running Android devices to install on");
            return;
        }
        for apk in apks {
            if !apk.is_file()
                || !apk
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("apk"))
            {
                self.toasts
                    .push(Notice::Error, "Enter the path to an existing .apk file");
                continue;
            }
            let apk = match std::path::absolute(&apk) {
                Ok(apk) => apk,
                Err(error) => {
                    self.toasts.push(
                        Notice::Error,
                        format!("Could not resolve APK path: {error}"),
                    );
                    continue;
                }
            };
            let id = self.toasts.install(apk.clone(), targets.clone());
            for (serial, _) in &targets {
                self.enqueue_install(
                    ctx,
                    serial.clone(),
                    InstallJob {
                        id,
                        apk: apk.clone(),
                        downgrade: false,
                    },
                );
            }
        }
    }

    fn enqueue_install(&mut self, ctx: &egui::Context, serial: String, job: InstallJob) {
        let queue = self.install_queue.entry(serial.clone()).or_default();
        queue.push_back(job);
        if queue.len() == 1 {
            self.begin_install(ctx, &serial);
        }
    }

    fn begin_install(&mut self, ctx: &egui::Context, serial: &str) {
        let Some(job) = self
            .install_queue
            .get(serial)
            .and_then(|queue| queue.front())
        else {
            return;
        };
        self.toasts.begin_install(job.id, serial);
        let (id, apk, downgrade) = (job.id, job.apk.clone(), job.downgrade);
        let (sdk, tx, ctx, serial) = (
            self.sdk.clone(),
            self.tx.clone(),
            ctx.clone(),
            serial.to_owned(),
        );
        thread::spawn(move || {
            let result = devices::install(&sdk, &serial, &apk, downgrade);
            let _ = tx.send(Msg::InstallDone(id, serial, result));
            ctx.request_repaint();
        });
    }

    fn finish_install(
        &mut self,
        ctx: &egui::Context,
        id: u64,
        serial: &str,
        result: Result<(), devices::InstallFailure>,
    ) {
        let Some(queue) = self.install_queue.get_mut(serial) else {
            return;
        };
        if queue.front().is_none_or(|job| job.id != id) {
            return;
        }
        self.toasts.finish_install(id, serial, result);
        queue.pop_front();
        if queue.is_empty() {
            self.install_queue.remove(serial);
        } else {
            self.begin_install(ctx, serial);
        }
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
    fn sample_fps(&mut self, ctx: &egui::Context) {
        let elapsed = self.fps_at.elapsed();
        if elapsed >= Duration::from_secs(1) {
            self.fps_at = Instant::now();
            for (serial, control) in self.capture.controls() {
                let count = control.frames.load(Ordering::Relaxed);
                let fps = self
                    .fps
                    .get(serial)
                    .and_then(|&(last, _)| frame_rate(last, count, elapsed));
                self.fps.insert(serial.clone(), (count, fps));
            }
        }
        if !self.fps.is_empty() || self.capture.controls().next().is_some() {
            ctx.request_repaint_after(Duration::from_secs(1).saturating_sub(self.fps_at.elapsed()));
        }
    }

    fn fps(&self, serial: &str) -> Option<u32> {
        self.fps.get(serial).and_then(|&(_, fps)| fps)
    }

    fn status_badge(&self, d: &Device) -> (String, Tone) {
        match &d.state {
            State::Running { .. } if d.platform == Platform::Ios => {
                ("VIEW ONLY".into(), Tone::Polled)
            }
            State::Running { serial } => {
                let control = self.capture.control(serial);
                let live = control.is_some_and(|c| c.is_live());
                match self.fps(serial) {
                    Some(fps) if live => (format!("LIVE {fps} fps"), Tone::Live),
                    Some(fps) => (format!("{fps} fps"), Tone::Polled),
                    None if live => ("LIVE".into(), Tone::Live),
                    None if self.screens.contains_key(serial) => ("POLLING".into(), Tone::Polled),
                    None => ("STARTING".into(), Tone::Busy),
                }
            }
            State::Booting { .. } => ("BOOTING".into(), Tone::Busy),
            State::Other { status, .. } => (status.to_uppercase(), Tone::Busy),
            State::Stopped => ("STOPPED".into(), Tone::Stopped),
        }
    }

    fn transport(&self, d: &Device) -> &'static str {
        if d.platform == Platform::Ios {
            return "simctl · view only";
        }
        let control = d.serial().and_then(|serial| self.capture.control(serial));
        match control {
            Some(c) if c.live.load(Ordering::Relaxed) => "gRPC",
            Some(c) if c.adb_live.load(Ordering::Relaxed) => "adb · live",
            Some(c) if c.frames.load(Ordering::Relaxed) > 0 => "adb · PNG",
            _ => "starting",
        }
    }
}

fn device_kind(d: &Device) -> String {
    let kind = match (d.platform, d.is_emulator) {
        (Platform::Ios, _) => "Simulator",
        (Platform::Android, true) => "Emulator",
        (Platform::Android, false)
            if d.known_serial().is_some_and(|serial| serial.contains(':')) =>
        {
            "Wi-Fi device"
        }
        (Platform::Android, false) => "USB device",
    };
    d.os.as_deref()
        .map_or_else(|| kind.into(), |os| format!("{os} · {kind}"))
}

fn matches_filter(d: &Device, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    [
        Some(d.display_name.as_str()),
        d.known_serial(),
        d.avd.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|value| value.to_lowercase().contains(&query))
}

fn frame_rate(last: u64, count: u64, elapsed: Duration) -> Option<u32> {
    if elapsed.is_zero() || count < last {
        return None;
    }
    Some(((count - last) as f64 / elapsed.as_secs_f64()).round() as u32)
}

impl eframe::App for EmuGrid {
    /// While a device is focused the keyboard belongs to it: take Tab before
    /// egui turns it into widget focus, and make sure no button holds focus
    /// (Enter / Space would press it).
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        let focused = self.focused.is_some()
            && self.view == View::Grid
            && self.dialog.is_none()
            && !self.details_open;
        self.editing_text = ctx.text_edit_focused();
        self.popup_has_keys = ctx.any_popup_open();
        if self.dialog.is_none()
            && !self.details_open
            && !self.sidebar_overlay
            && !self.popup_has_keys
        {
            self.key_commands.extend(shortcuts::take(
                &mut raw.events,
                focused && !self.editing_text,
            ));
        }
        if !focused || self.editing_text || self.popup_has_keys {
            self.pending_tabs = 0;
            return;
        }
        self.pending_tabs += input::take_tabs(&mut raw.events);
        ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
    }

    // eframe still runs logic while a hidden window skips drawing.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let (hidden, focused) = ctx.input(|i| (viewport_hidden(i), i.focused));
        if hidden || !focused {
            self.lift_finger();
        }
        if hidden {
            for (_, c) in self.capture.controls() {
                c.set_active(false);
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = &ui.ctx().clone();
        #[cfg(target_os = "linux")]
        if self
            .theme_check
            .is_none_or(|last| last.elapsed() >= Duration::from_secs(15))
        {
            self.theme_check = Some(Instant::now());
            let (tx, ctx) = (self.tx.clone(), ctx.clone());
            thread::spawn(move || {
                let _ = tx.send(Msg::SystemTheme(theme::linux_system_theme()));
                ctx.request_repaint();
            });
        }
        let full = ui.max_rect();

        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Scan(generation, list, eps) => {
                    if generation == self.sdk_generation {
                        self.on_scan(list, eps);
                    }
                }
                Msg::Status(result) => {
                    match result {
                        Ok(s) => self.message = Some(s),
                        Err(error) => self.toasts.push(Notice::Error, error),
                    }
                    self.last_scan = None;
                }
                Msg::InputError(s) => self.toasts.push(Notice::Error, s),
                Msg::InstallDone(id, serial, result) => {
                    self.finish_install(ctx, id, &serial, result)
                }
                Msg::ScreenSize(serial, ticket, size) => {
                    if self.size_pending.get(&serial) == Some(&ticket) {
                        self.size_pending.remove(&serial);
                        if let Some(size) = size {
                            self.screen_sizes.insert(serial, size);
                        }
                    }
                }
                #[cfg(target_os = "linux")]
                Msg::SystemTheme(system) => {
                    ctx.set_theme(system.map_or(egui::ThemePreference::System, Into::into));
                }
            }
        }
        self.sample_fps(ctx);
        let frames: Vec<_> = self.capture.frames().collect();
        for (serial, f) in frames {
            let size = [f.width as usize, f.height as usize];
            let limit = ctx.input(|i| i.max_texture_side);
            if size[0].max(size[1]) > limit {
                if self.focused.as_deref() == Some(&serial) && self.zoom != Zoom::Fit {
                    self.set_zoom(Zoom::Fit);
                    self.toasts.push(
                        Notice::Info,
                        "This display exceeds the renderer’s native texture size",
                    );
                }
                continue;
            }
            let options = if self.focused.as_deref() == Some(&serial) {
                self.zoom.texture_options()
            } else {
                egui::TextureOptions::LINEAR
            };
            let image = egui::ColorImage::from_rgb(size, &f.rgb);
            let first_pixel = image.pixels[0];
            match self.screens.get_mut(&serial) {
                Some(s) => {
                    s.tex.set(image, options);
                    s.size = size;
                    s.first_pixel = first_pixel;
                }
                None => {
                    let tex = ctx.load_texture(&serial, image, options);
                    self.screens.insert(
                        serial,
                        Screen {
                            tex,
                            size,
                            first_pixel,
                        },
                    );
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

        self.launches.retain(|avd, child| {
            if let Ok(Some(status)) = child.try_wait() {
                if !status.success() && self.starts.remove(avd).is_some() {
                    self.toasts.push(
                        Notice::Error,
                        format!("{avd} exited before boot completed ({status})"),
                    );
                }
                self.last_scan = None;
                false
            } else {
                true
            }
        });
        // A launcher can exit successfully after handing off to the emulator.
        // Keep its pending tile while waiting for the new serial.
        let pending = self.starts.len();
        self.starts.retain(|avd, (started, _)| {
            self.launches.contains_key(avd) || started.elapsed() < BOOT_DISCOVERY_GRACE
        });
        if self.starts.len() != pending {
            self.last_scan = None;
        }
        if self.last_scan.is_none_or(|t| t.elapsed() >= REFRESH_EVERY) {
            self.scan(ctx);
        }
        ctx.request_repaint_after(REFRESH_EVERY);

        let mut actions = Vec::new();
        for command in std::mem::take(&mut self.key_commands) {
            if command == KeyCommand::Filter {
                self.focus_filter = true;
                if self.focused.is_some() {
                    self.apply(ctx, Action::Unfocus);
                }
            } else if let Some(serial) = self.focused.as_deref()
                && let Some(action) = self.device_shortcut(serial, command)
            {
                actions.push(action);
            }
        }
        self.visible.clear();

        let p = theme::palette(ctx);
        self.toolbar(ui, &mut actions);
        self.status_bar(ui);
        self.sidebar(ui, &mut actions);

        egui::CentralPanel::default_margins()
            .frame(
                egui::Frame::new()
                    .fill(p.bg)
                    .inner_margin(egui::Margin::same(16)),
            )
            .show(ui, |ui| match (self.view, self.focused.clone()) {
                (View::Grid, Some(serial)) => {
                    let had_details = self.details_open;
                    self.focused_view(ui, &serial, &mut actions);
                    if self.dialog.is_none() && !self.details_open && !had_details {
                        self.forward_keys(ctx, &serial, &mut actions);
                    }
                }
                (View::Grid, None) => self.grid_view(ui, &mut actions),
                (View::List, _) => self.list_view(ui, &mut actions),
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

        self.show_dialog(ctx, &mut actions);
        self.toasts.show(ctx, &mut actions);
        for a in actions {
            self.apply(ctx, a);
        }

        if self.visible != self.last_grid_visible {
            self.last_grid_visible.clone_from(&self.visible);
            ctx.request_repaint();
        }
        // Only stream what is on screen; nothing at all while minimized.
        let hidden = ctx.input(viewport_hidden);
        for (serial, c) in self.capture.controls() {
            let focused = self.focused.as_deref() == Some(serial.as_str());
            c.set_focused(focused);
            c.set_active(
                !hidden && (focused || (self.focused.is_none() && self.visible.contains(serial))),
            );
        }
    }
}

fn viewport_hidden(input: &egui::InputState) -> bool {
    let viewport = input.viewport();
    viewport.visible() == Some(false) || viewport.minimized == Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fps_uses_elapsed_time_and_handles_reset() {
        assert_eq!(frame_rate(10, 55, Duration::from_millis(1500)), Some(30));
        assert_eq!(frame_rate(10, 10, Duration::from_secs(1)), Some(0));
        assert_eq!(frame_rate(10, 1, Duration::from_secs(1)), None);
    }

    #[test]
    fn device_filter_matches_display_name_serial_and_avd_case_insensitively() {
        let d = Device {
            display_name: "Pixel 8".into(),
            api: Some(35),
            os: Some("Android 15".into()),
            platform: Platform::Android,
            is_emulator: true,
            avd: Some("Pixel_API_35".into()),
            state: State::Booting {
                serial: Some("emulator-5554".into()),
            },
            udid: None,
        };
        for query in ["", " PIXEL ", "Emulator-5554", "API_35"] {
            assert!(matches_filter(&d, query));
        }
        assert!(!matches_filter(&d, "iPhone"));
    }

    #[test]
    fn a_local_start_appears_booting_before_adb_sees_it() {
        let ctx = egui::Context::default();
        let mut app = EmuGrid::new(&ctx);
        app.starts
            .insert("Test_AVD".into(), (Instant::now(), Boot::Cold));
        app.on_scan(
            vec![Device {
                display_name: "Test Device".into(),
                api: Some(35),
                os: Some("Android 15".into()),
                platform: Platform::Android,
                is_emulator: true,
                avd: Some("Test_AVD".into()),
                state: State::Stopped,
                udid: None,
            }],
            HashMap::new(),
        );
        assert_eq!(app.devices[0].state, State::Booting { serial: None });
        assert_eq!(app.running().count(), 0);
    }

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
        theme::install(&ctx);
        let mut out = ctx.run_ui(Default::default(), |_| {});
        out.textures_delta.clear();
        let sources = [
            include_str!("app.rs"),
            include_str!("widgets.rs"),
            include_str!("ui/grid.rs"),
            include_str!("ui/focused.rs"),
            include_str!("ui/list.rs"),
            include_str!("ui/setup.rs"),
            include_str!("ui/sidebar.rs"),
            include_str!("ui/toasts.rs"),
            include_str!("ui/toolbar.rs"),
            include_str!("shortcuts.rs"),
        ];
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
        for width in [328.0, 465.0, 550.0, 620.0, 800.0] {
            let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 58.0));
            let cols = list::device_columns(rect);
            assert_eq!(cols[1].width() > 0.0, width > 465.0);
            assert_eq!(cols[3].width() > 0.0, width > 620.0);
            assert_eq!(cols[0].left(), rect.left());
            assert_eq!(cols[4].right(), rect.right());
            for pair in cols.windows(2) {
                assert_eq!(pair[0].right(), pair[1].left());
            }
        }
    }

    #[test]
    fn focused_device_without_frame_still_draws_a_bezel() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut app = EmuGrid::new(&ctx);
        app.devices = vec![Device {
            display_name: "phone".into(),
            api: None,
            platform: Platform::Android,
            is_emulator: true,
            state: State::Running {
                serial: "emulator-5554".into(),
            },
            avd: None,
            os: None,
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
        theme::install(&ctx);
        let mut app = EmuGrid::new(&ctx);
        app.devices = vec![Device {
            display_name: "phone".into(),
            api: None,
            platform: Platform::Android,
            is_emulator: true,
            state: State::Running {
                serial: "emulator-5554".into(),
            },
            avd: None,
            os: None,
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
                first_pixel: Color32::BLACK,
            },
        );
        app.screen_sizes
            .insert("emulator-5554".into(), (1080, 2400));
        app.focused = Some("emulator-5554".into());

        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 700.0))),
            ..Default::default()
        };
        let mut frame = ctx.run_ui(raw, |ui| {
            app.focused_view(ui, "emulator-5554", &mut Vec::new());
        });
        let center = frame
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect)
                    if rect.fill == crate::theme::BEZEL && rect.rect.width() > 70.0 =>
                {
                    Some(rect.rect.center())
                }
                _ => None,
            })
            .expect("focused bezel");
        frame.textures_delta.clear();
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
