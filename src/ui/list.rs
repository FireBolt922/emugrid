use super::*;
use egui::Ui;

#[derive(Clone, Copy, Default, PartialEq)]
pub(super) enum DeviceTab {
    #[default]
    All,
    Emulators,
    Physical,
    Ios,
}

impl DeviceTab {
    fn includes(self, d: &Device) -> bool {
        match self {
            Self::All => true,
            Self::Emulators => d.platform == Platform::Android && d.is_emulator,
            Self::Physical => d.platform == Platform::Android && !d.is_emulator,
            Self::Ios => d.platform == Platform::Ios,
        }
    }
}

impl EmuGrid {
    pub(super) fn list_view(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let p = widgets::pal(ui);
        let booting = self
            .devices
            .iter()
            .filter(|d| matches!(d.state, State::Booting { .. }))
            .count();
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Devices").font(theme::semibold(20.0)));
            ui.label(
                RichText::new(format!(
                    "{} found · {} running · {booting} booting",
                    self.devices.len(),
                    self.running().count()
                ))
                .size(11.5)
                .color(p.text3),
            );
        });
        self.filter_field(ui, "Search name or serial", 156.0);
        ui.horizontal_wrapped(|ui| {
            for (tab, label) in [
                (DeviceTab::All, "All"),
                (DeviceTab::Emulators, "Emulators"),
                (DeviceTab::Physical, "Physical"),
                (DeviceTab::Ios, "iOS"),
            ] {
                let n = self.devices.iter().filter(|d| tab.includes(d)).count();
                if ui
                    .selectable_label(self.device_tab == tab, format!("{label} {n}"))
                    .clicked()
                {
                    self.device_tab = tab;
                }
            }
        });
        egui::ScrollArea::vertical().show(ui, |ui| {
            let mut any = false;
            for (tab, group) in [
                (DeviceTab::Emulators, "Android emulators"), (DeviceTab::Physical, "Physical devices"), (DeviceTab::Ios, "iOS simulators"),
            ] {
                if self.device_tab != DeviceTab::All && self.device_tab != tab {
                    continue;
                }
                let rows: Vec<_> = self.devices.iter().filter(|d| tab.includes(d) && matches_filter(d, &self.filter)).collect();
                if !rows.is_empty() {
                    any = true;
                    device_group(ui, group, &rows, actions);
                } else if tab == DeviceTab::Ios && !self.sdk.simctl {
                    any |= self.device_tab == DeviceTab::Ios;
                    let info = if cfg!(target_os = "macos") {
                        "Install Xcode to see iOS simulators here."
                    } else {
                        "iOS simulators need macOS with Xcode. On a Mac they show up here automatically."
                    };
                    egui::Frame::new().fill(p.chrome).corner_radius(RADIUS_CARD).inner_margin(9).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            widgets::icon(ui, Icon::Info, 14.0, p.text3);
                            ui.add(egui::Label::new(RichText::new(info).size(11.0).color(p.text2)).wrap());
                        });
                    });
                }
            }
            if !any && !self.scanning {
                ui.label(RichText::new(if self.filter.trim().is_empty() { "No devices found" } else { "No devices match the search" }).color(p.text3));
            }
        });
    }
}

// Shared column rectangles keep headings and values aligned at every width.
pub(super) fn device_columns(rect: Rect) -> [Rect; 5] {
    let width = rect.width();
    let actions = 172.0_f32.min(width * 0.42);
    let state = 134.0_f32.min((width - actions) * 0.49);
    let serial = if width > 620.0 {
        155.0_f32.min(width * 0.23)
    } else {
        0.0
    };
    let system = if width > 465.0 {
        141.0_f32.min(width * 0.21)
    } else {
        0.0
    };
    let x = [
        rect.left(),
        rect.right() - actions - serial - state - system,
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

fn cell(ui: &mut Ui, rect: Rect, layout: egui::Layout) -> Ui {
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(7.0, 0.0)))
            .layout(layout),
    );
    child.set_clip_rect(ui.clip_rect().intersect(rect));
    child
}

fn device_group(ui: &mut Ui, title: &str, rows: &[&Device], actions: &mut Vec<Action>) {
    let p = widgets::pal(ui);
    egui::Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(RADIUS_CARD)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            let (band, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::hover());
            ui.painter().rect_filled(
                band,
                egui::CornerRadius {
                    nw: 13,
                    ne: 13,
                    sw: 0,
                    se: 0,
                },
                p.chrome,
            );
            ui.painter().text(
                band.left_center() + Vec2::new(9.0, 0.0),
                egui::Align2::LEFT_CENTER,
                format!("{title} · {}", rows.len()),
                theme::semibold(11.5),
                p.text2,
            );

            let (header, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 29.0), Sense::hover());
            for (rect, title) in device_columns(header)
                .into_iter()
                .zip(["Device", "System", "State", "Serial", "Actions"])
            {
                if rect.width() > 0.0 {
                    let layout = if title == "Actions" {
                        egui::Layout::right_to_left(egui::Align::Center)
                    } else {
                        egui::Layout::left_to_right(egui::Align::Center)
                    };
                    widgets::section_label(&mut cell(ui, rect, layout), title);
                }
            }
            for d in rows {
                let (row, _) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::hover());
                if !ui.is_rect_visible(row) {
                    continue;
                }
                ui.painter()
                    .hline(row.x_range(), row.top(), Stroke::new(1.0, p.border));
                ui.push_id(
                    (d.avd.as_deref(), d.known_serial(), d.udid.as_deref()),
                    |ui| device_row(ui, row, d, actions),
                );
            }
        });
}

fn device_row(ui: &mut Ui, row: Rect, d: &Device, actions: &mut Vec<Action>) {
    let p = widgets::pal(ui);
    let (pill, tone, detail) = match &d.state {
        State::Running { .. } if d.platform == Platform::Ios => ("BOOTED", Tone::Live, None),
        State::Running { .. } if !d.is_emulator => (
            "Connected",
            Tone::Polled,
            Some(
                if d.known_serial().is_some_and(|serial| serial.contains(':')) {
                    "Wi-Fi · screen at ~2 fps"
                } else {
                    "USB · screen at ~2 fps"
                },
            ),
        ),
        State::Running { .. } => ("RUNNING", Tone::Live, None),
        State::Booting { .. } => ("BOOTING", Tone::Busy, Some("Waiting for boot…")),
        State::Other { status, .. } if matches!(status.as_str(), "unauthorized" | "offline") => {
            (status.as_str(), Tone::Busy, Some("Allow USB debugging"))
        }
        State::Other { status, .. } => (status.as_str(), Tone::Busy, None),
        State::Stopped if d.platform == Platform::Ios => ("SHUT DOWN", Tone::Stopped, None),
        State::Stopped => ("STOPPED", Tone::Stopped, None),
    };
    let layout = egui::Layout::left_to_right(egui::Align::Center);
    let columns = device_columns(row);
    let mut device = cell(ui, columns[0], layout);
    device.spacing_mut().item_spacing.x = 5.0;
    let (dot, _) = device.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
    widgets::paint_dot(
        device.painter(),
        dot.center(),
        tone.colors(p).0,
        tone == Tone::Stopped,
    );
    device.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.add(
            egui::Label::new(RichText::new(&d.display_name).font(theme::semibold(11.5))).truncate(),
        )
        .on_hover_text(d.title());
        if let Some(id) = d.avd.as_deref().or_else(|| {
            (columns[3].width() == 0.0)
                .then(|| d.known_serial())
                .flatten()
        }) {
            ui.add(
                egui::Label::new(RichText::new(id).monospace().size(9.5).color(p.text3)).truncate(),
            )
            .on_hover_text(id);
        }
    });
    for (index, text, mono) in [
        (1, d.os.as_deref().unwrap_or("—"), false),
        (
            3,
            d.known_serial().or(d.udid.as_deref()).unwrap_or("—"),
            true,
        ),
    ] {
        if columns[index].width() > 0.0 {
            let label = RichText::new(text).size(11.0).color(p.text2);
            cell(ui, columns[index], layout)
                .add(egui::Label::new(if mono { label.monospace() } else { label }).truncate())
                .on_hover_text(text);
        }
    }
    let mut state = cell(ui, columns[2], layout);
    if let Some(detail) = detail {
        state.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            widgets::status_pill(ui, pill, tone);
            ui.add(egui::Label::new(RichText::new(detail).size(9.0).color(p.text3)).truncate())
                .on_hover_text(detail);
        });
    } else {
        widgets::status_pill(&mut state, pill, tone);
    }
    let mut buttons = cell(
        ui,
        columns[4],
        egui::Layout::right_to_left(egui::Align::Center),
    );
    row_actions(&mut buttons, d, actions);
}

fn row_actions(ui: &mut Ui, d: &Device, actions: &mut Vec<Action>) {
    match (&d.state, d.platform) {
        (State::Stopped, Platform::Android) => {
            let Some(avd) = &d.avd else { return };
            let start = widgets::split_button(ui, "Start", |ui| {
                for (title, detail, boot) in [
                    ("Start", "Resume from the saved snapshot", Boot::Normal),
                    ("Cold boot", "Ignore the saved snapshot", Boot::Cold),
                ] {
                    if widgets::menu_item(ui, title, detail, false).clicked() {
                        actions.push(Action::Start(avd.clone(), boot));
                        ui.close();
                    }
                }
                ui.separator();
                if widgets::menu_item(
                    ui,
                    "Wipe data and start…",
                    "Erase all apps and files on this AVD",
                    true,
                )
                .clicked()
                {
                    actions.push(Action::Dialog(Dialog::Wipe {
                        avd: avd.clone(),
                        name: d.display_name.clone(),
                    }));
                    ui.close();
                }
            });
            if start.clicked() {
                actions.push(Action::Start(avd.clone(), Boot::Normal));
            }
        }
        (State::Stopped, Platform::Ios) => {
            if let Some(udid) = &d.udid
                && widgets::button(ui, Kind::Primary, Some(Icon::Play), "Boot").clicked()
            {
                actions.push(Action::IosBoot(udid.clone()));
            }
        }
        (State::Booting { serial }, Platform::Android) if d.is_emulator => {
            if widgets::button(ui, Kind::Ghost, Some(Icon::Stop), "Stop").clicked() {
                if let Some(serial) = serial {
                    actions.push(Action::Stop(serial.clone()));
                } else if let Some(avd) = &d.avd {
                    actions.push(Action::StopLaunch(avd.clone()));
                }
            }
        }
        (State::Running { serial }, _) => {
            if d.is_emulator && widgets::button(ui, Kind::Ghost, None, "Stop").clicked() {
                actions.push(d.udid.as_ref().map_or_else(
                    || Action::Stop(serial.clone()),
                    |udid| Action::IosShutdown(udid.clone()),
                ));
            }
            if widgets::button(ui, Kind::Secondary, None, "Open").clicked() {
                actions.push(Action::Focus(serial.clone()));
            }
        }
        _ => {}
    }
}
