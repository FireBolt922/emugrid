//! The egui window: a live list of emulators with start / stop buttons.

use crate::devices::{self, Device, State};
use crate::sdk::Sdk;
use eframe::egui;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

const REFRESH_EVERY: Duration = Duration::from_secs(3);

pub struct EmuGrid {
    sdk: Sdk,
    devices: Vec<Device>,
    tx: Sender<Vec<Device>>,
    rx: Receiver<Vec<Device>>,
    scanning: bool,
    last_scan: Option<Instant>,
    message: Option<String>,
}

impl EmuGrid {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            sdk: Sdk::detect(),
            devices: Vec::new(),
            tx,
            rx,
            scanning: false,
            last_scan: None,
            message: None,
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
            let _ = tx.send(devices::scan(&sdk));
            ctx.request_repaint();
        });
    }
}

impl eframe::App for EmuGrid {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = &ui.ctx().clone();
        if let Ok(list) = self.rx.try_recv() {
            self.devices = list;
            self.scanning = false;
        }
        if self.last_scan.is_none_or(|t| t.elapsed() >= REFRESH_EVERY) {
            self.scan(ctx);
        }
        ctx.request_repaint_after(REFRESH_EVERY);

        egui::Panel::top("top").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("emugrid");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("⟳ Refresh").clicked() {
                        self.last_scan = None;
                    }
                    if self.scanning {
                        ui.spinner();
                    }
                });
            });
        });

        egui::Panel::bottom("status").show(ui, |ui| {
            let running = self
                .devices
                .iter()
                .filter(|d| matches!(d.state, State::Running { .. }))
                .count();
            ui.horizontal(|ui| {
                ui.label(format!(
                    "{} devices · {running} running",
                    self.devices.len()
                ));
                if let Some(msg) = &self.message {
                    ui.separator();
                    ui.label(msg);
                }
            });
        });

        egui::CentralPanel::default_margins().show(ui, |ui| {
            if self.sdk.adb.is_none() && self.sdk.emulator.is_none() {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "Android SDK not found. Set ANDROID_HOME or put adb and emulator on your PATH.",
                );
                return;
            }
            if self.devices.is_empty() && !self.scanning {
                ui.label("No emulators or devices found.");
            }

            let mut action: Option<Result<String, String>> = None;
            egui::ScrollArea::vertical().show(ui, |ui| {
                for d in &self.devices {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            let (dot, color) = match &d.state {
                                State::Running { .. } => {
                                    ("●", egui::Color32::from_rgb(60, 180, 90))
                                }
                                State::Other { .. } => ("●", egui::Color32::from_rgb(220, 160, 40)),
                                State::Stopped => ("○", ui.visuals().weak_text_color()),
                            };
                            ui.colored_label(color, dot);
                            ui.vertical(|ui| {
                                ui.strong(&d.name);
                                let kind = if d.is_emulator {
                                    "Emulator"
                                } else {
                                    "Physical device"
                                };
                                let detail = match &d.state {
                                    State::Running { serial } => {
                                        format!("{kind} · running · {serial}")
                                    }
                                    State::Other { serial, status } => {
                                        format!("{kind} · {status} · {serial}")
                                    }
                                    State::Stopped => format!("{kind} · stopped"),
                                };
                                ui.small(detail);
                            });
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| match &d.state {
                                    State::Stopped if ui.button("▶ Start").clicked() => {
                                        action = Some(
                                            devices::start(&self.sdk, &d.name)
                                                .map(|_| format!("Starting {}…", d.name)),
                                        );
                                    }
                                    State::Running { serial }
                                        if d.is_emulator && ui.button("■ Stop").clicked() =>
                                    {
                                        action = Some(
                                            devices::stop(&self.sdk, serial)
                                                .map(|_| format!("Stopping {}…", d.name)),
                                        );
                                    }
                                    _ => {}
                                },
                            );
                        });
                    });
                }
            });

            if let Some(result) = action {
                self.message = Some(result.unwrap_or_else(|e| format!("Error: {e}")));
                self.last_scan = None;
            }
        });
    }
}
