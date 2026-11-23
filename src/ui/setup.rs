use super::*;

impl EmuGrid {
    pub(super) fn empty_grid(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let blocked = self.sdk.adb.is_none() && self.sdk.emulator.is_none() && !self.sdk.simctl;
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            if blocked {
                self.sdk_missing(ui, actions);
                return;
            }
            let p = widgets::pal(ui);
            ui.add_space(10.0);
            ui.vertical_centered(|ui| {
                widgets::icon(ui, Icon::Grid, 32.0, p.accent);
                if !self.scanned && self.devices.is_empty() {
                    ui.heading("Checking for devices…");
                } else {
                    ui.heading("No devices running");
                    ui.label(RichText::new("Start an emulator and its screen streams here live. Phones plugged in over USB show up too.").color(p.text2));
                }
            });
            ui.add_space(12.0);
            let avds: Vec<_> = self.devices.iter()
                .filter(|d| d.avd.is_some() && d.state == State::Stopped).collect();
            ui.horizontal(|ui| {
                widgets::section_label(ui, "Your emulators");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.link(format!("All {} in List", avds.len())).clicked() {
                        actions.push(Action::View(View::List));
                    }
                });
            });
            if avds.is_empty() {
                ui.label(RichText::new("No stopped Android emulators found").color(p.text3));
            } else {
                let columns = if ui.available_width() >= 470.0 { 2 } else { 1 };
                ui.columns(columns, |cells| {
                    for (index, device) in avds.into_iter().enumerate() {
                        let ui = &mut cells[index % columns];
                        avd_card(ui, device, actions);
                    }
                });
            }
            ui.add_space(10.0);
            self.environment(ui);
            if self.sdk.adb.is_none() {
                ui.add_space(8.0);
                self.sdk_missing(ui, actions);
            }
        });
    }

    fn environment(&self, ui: &mut egui::Ui) {
        let p = widgets::pal(ui);
        egui::Frame::new()
            .fill(p.surface)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(RADIUS_CARD)
            .inner_margin(14)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                widgets::section_label(ui, "Environment");
                let items = [
                    ("Android SDK", self.sdk.root.as_deref()),
                    ("adb", self.sdk.adb.as_deref()),
                    ("emulator", self.sdk.emulator.as_deref()),
                ];
                for (label, path) in items {
                    environment_row(
                        ui,
                        label,
                        path.map_or_else(|| "Not found".into(), |path| path.display().to_string()),
                        path.is_some(),
                    );
                }
                if cfg!(target_os = "macos") {
                    let n = self
                        .devices
                        .iter()
                        .filter(|d| d.platform == Platform::Ios)
                        .count();
                    let value = if self.sdk.simctl {
                        format!("simctl · {n} simulators")
                    } else {
                        "simctl not found".into()
                    };
                    environment_row(ui, "iOS", value, self.sdk.simctl);
                }
            });
    }

    fn sdk_missing(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let p = widgets::pal(ui);
        egui::Frame::new()
            .fill(p.surface)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(RADIUS_CARD)
            .inner_margin(16)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.heading("Android SDK not found");
                ui.label(
                    RichText::new("adb is required to discover and stream Android devices.")
                        .color(p.text2),
                );
                for check in &self.sdk_checks {
                    ui.horizontal(|ui| {
                        widgets::icon(
                            ui,
                            if check.found {
                                Icon::Check
                            } else {
                                Icon::Close
                            },
                            14.0,
                            if check.found { p.live } else { p.text3 },
                        );
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            ui.add(
                                egui::Label::new(format!("{} · {}", check.label, check.result))
                                    .wrap(),
                            );
                            if let Some(path) = &check.path
                                && path.to_string_lossy() != check.label
                            {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(path.display().to_string())
                                            .monospace()
                                            .size(10.5)
                                            .color(p.text3),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(path.display().to_string());
                            }
                        });
                    });
                }
                self.sdk_folder_controls(ui, actions);
                ui.label("No SDK yet?");
                const COMMAND: &str = "sdkmanager \"platform-tools\" \"emulator\"";
                ui.horizontal(|ui| {
                    if widgets::button(ui, Kind::Secondary, Some(Icon::Copy), "Copy").clicked() {
                        ui.ctx().copy_text(COMMAND.into());
                        actions.push(Action::CopiedCommand);
                    }
                    ui.add(egui::Label::new(RichText::new(COMMAND).monospace()).wrap());
                });
            });
    }

    pub(super) fn sdk_folder_controls(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let mut opened = false;
        ui.horizontal_wrapped(|ui| {
            if self.sdk_path.is_none()
                && widgets::button(
                    ui,
                    Kind::Secondary,
                    Some(Icon::Folder),
                    "Choose SDK folder…",
                )
                .clicked()
            {
                opened = true;
                self.sdk_path = Some(
                    self.settings
                        .sdk_root
                        .as_deref()
                        .map(|path| path.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                );
            }
            if widgets::button(ui, Kind::Secondary, Some(Icon::Refresh), "Check again").clicked() {
                actions.push(Action::RecheckSdk);
            }
        });
        if let Some(path) = &mut self.sdk_path {
            let mut close = false;
            ui.horizontal_wrapped(|ui| {
                let edit = ui.add(
                    egui::TextEdit::singleline(path)
                        .hint_text("Android SDK folder")
                        .desired_width((ui.available_width() - 110.0).max(90.0)),
                );
                if opened {
                    edit.request_focus();
                }
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (widgets::button(ui, Kind::Primary, None, "Use").clicked() || enter)
                    && !path.trim().is_empty()
                {
                    actions.push(Action::UseSdk(PathBuf::from(path.trim())));
                }
                close = widgets::icon_button(ui, Icon::Close, "Cancel", 28.0, false).clicked();
            });
            if close {
                self.sdk_path = None;
            }
        }
    }
}

fn avd_card(ui: &mut egui::Ui, device: &Device, actions: &mut Vec<Action>) {
    let Some(avd) = &device.avd else {
        return;
    };
    let p = widgets::pal(ui);
    egui::Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(RADIUS_CARD)
        .inner_margin(10)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                widgets::icon(ui, Icon::Phone, 18.0, p.accent);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::button(ui, Kind::Secondary, Some(Icon::Play), "Start").clicked() {
                        actions.push(Action::Start(avd.clone(), Boot::Normal));
                    }
                    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.add(
                            egui::Label::new(
                                RichText::new(&device.display_name).font(theme::semibold(12.0)),
                            )
                            .truncate(),
                        )
                        .on_hover_text(device.title());
                        ui.add(
                            egui::Label::new(
                                RichText::new(device.os.as_deref().unwrap_or("Android"))
                                    .size(11.0)
                                    .color(p.text3),
                            )
                            .truncate(),
                        );
                    });
                });
            });
        });
}

fn environment_row(ui: &mut egui::Ui, label: &str, value: String, found: bool) {
    let p = widgets::pal(ui);
    ui.horizontal(|ui| {
        widgets::icon(
            ui,
            if found { Icon::Check } else { Icon::Close },
            15.0,
            if found { p.live } else { p.danger },
        );
        ui.add_sized([86.0, 18.0], egui::Label::new(label));
        ui.add(
            egui::Label::new(RichText::new(&value).monospace().size(10.5).color(p.text2))
                .truncate(),
        )
        .on_hover_text(value);
    });
}
