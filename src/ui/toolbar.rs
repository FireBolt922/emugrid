use super::*;

#[derive(Clone, PartialEq)]
pub(super) enum Dialog {
    Sdk,
    Shortcuts,
    About,
    Wipe { avd: String, name: String },
}

pub(super) const WIDE_TOOLBAR: f32 = if cfg!(target_os = "macos") {
    870.0
} else {
    770.0
};

impl EmuGrid {
    pub(super) fn toolbar(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let p = widgets::pal(ui);
        egui::Panel::top("top")
            .exact_size(52.0)
            .frame(
                egui::Frame::new()
                    .fill(p.chrome)
                    .stroke(Stroke::new(1.0, p.border))
                    .inner_margin(egui::Margin::symmetric(12, 0)),
            )
            .show(ui, |ui| {
                let wide = ui.ctx().content_rect().width() >= WIDE_TOOLBAR;
                let compact = ui.available_width() < 560.0;
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), 50.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        if cfg!(target_os = "macos") {
                            ui.add_space(60.0);
                        }
                        widgets::wordmark(ui, compact);
                        let selected = usize::from(self.view == View::List);
                        let items = [(Icon::Grid, "Grid"), (Icon::List, "List")];
                        let views = widgets::segmented(ui, &items, selected, compact);
                        if let Some(index) = views {
                            actions.push(Action::View(if index == 0 {
                                View::Grid
                            } else {
                                View::List
                            }));
                        }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            self.main_menu(ui, actions);
                            let install = if compact {
                                widgets::icon_button(ui, Icon::Upload, "Install APK", 30.0, true)
                            } else {
                                widgets::button(
                                    ui,
                                    Kind::Primary,
                                    Some(Icon::Upload),
                                    "Install APK",
                                )
                            };
                            egui::Popup::from_toggle_button_response(&install)
                                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                                .show(|ui| {
                                    ui.set_width(250.0);
                                    let targets = self.android_count();
                                    let field = ui.add(
                                        egui::TextEdit::singleline(&mut self.apk_path)
                                            .desired_width(f32::INFINITY)
                                            .hint_text("Path to .apk"),
                                    );
                                    if install.clicked() {
                                        field.request_focus();
                                    }
                                    let enabled = targets > 0 && !self.apk_path.trim().is_empty();
                                    let enter = field.lost_focus()
                                        && ui.input(|i| i.key_pressed(egui::Key::Enter));
                                    let label = format!(
                                        "Install on {targets} {}",
                                        if targets == 1 { "device" } else { "devices" }
                                    );
                                    let clicked = ui
                                        .add_enabled_ui(enabled, |ui| {
                                            widgets::button(ui, Kind::Primary, None, &label)
                                                .clicked()
                                        })
                                        .inner;
                                    if enabled && (clicked || enter) {
                                        actions.push(Action::Install(PathBuf::from(
                                            self.apk_path.trim(),
                                        )));
                                        ui.close();
                                    }
                                    ui.small("or drop an .apk anywhere");
                                });
                            if widgets::icon_button(
                                ui,
                                Icon::Refresh,
                                "Refresh devices",
                                30.0,
                                true,
                            )
                            .clicked()
                            {
                                actions.push(Action::Refresh);
                            }
                            if self.view == View::Grid && self.focused.is_none() {
                                if wide {
                                    self.tile_size_control(ui, actions);
                                } else {
                                    let size = widgets::icon_button(
                                        ui,
                                        Icon::TileSize,
                                        "Tile size",
                                        30.0,
                                        true,
                                    );
                                    egui::Popup::from_toggle_button_response(&size)
                                        .close_behavior(
                                            egui::PopupCloseBehavior::CloseOnClickOutside,
                                        )
                                        .show(|ui| {
                                            self.tile_size_control(ui, actions);
                                        });
                                }
                                if wide {
                                    ui.allocate_ui_with_layout(
                                        Vec2::new(217.0, 30.0),
                                        egui::Layout::left_to_right(egui::Align::Center),
                                        |ui| self.filter_field(ui, "Filter devices", 121.0),
                                    );
                                }
                            }
                            if cfg!(target_os = "macos") {
                                let rect = ui.available_rect_before_wrap();
                                if rect.width() > 2.0 {
                                    let drag = ui.allocate_rect(rect, Sense::click_and_drag());
                                    if drag.double_clicked() {
                                        let maximized =
                                            ui.input(|i| i.viewport().maximized.unwrap_or(false));
                                        ui.ctx().send_viewport_cmd(
                                            egui::ViewportCommand::Maximized(!maximized),
                                        );
                                    } else if drag.drag_started() {
                                        ui.ctx()
                                            .send_viewport_cmd(egui::ViewportCommand::StartDrag);
                                    }
                                }
                            }
                        });
                    },
                );
            });
    }

    fn tile_size_control(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.spacing_mut().slider_width = 76.0;
        widgets::icon(ui, Icon::TileSize, 16.0, widgets::pal(ui).text3);
        let slider = ui
            .add(egui::Slider::new(&mut self.settings.tile_width, 140..=320).show_value(false))
            .on_hover_text(format!("Tile size · {} pt", self.settings.tile_width));
        self.tile_size_dirty |= slider.changed();
        if self.tile_size_dirty && !slider.is_pointer_button_down_on() {
            self.tile_size_dirty = false;
            actions.push(Action::SaveSettings);
        }
    }

    pub(super) fn filter_field(&mut self, ui: &mut egui::Ui, hint: &str, width: f32) {
        let p = widgets::pal(ui);
        ui.horizontal(|ui| {
            widgets::icon(ui, Icon::Search, 14.0, p.text3);
            let id = ui.scope_id().with("device-filter");
            let focus = std::mem::take(&mut self.focus_filter);
            if focus {
                ui.memory_mut(|m| m.request_focus(id));
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape))
                && self.focused.is_none()
                && self.dialog.is_none()
                && !self.sidebar_overlay
                && !self.popup_has_keys
            {
                self.filter.clear();
                ui.memory_mut(|m| m.surrender_focus(id));
            }
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .id(id)
                    .hint_text(hint)
                    .desired_width(width)
                    .min_size(Vec2::new(0.0, 28.0)),
            );
            if ui.available_width() > 48.0 {
                widgets::kbd(ui, &shortcuts::label(KeyCommand::Filter));
            }
        });
    }

    fn main_menu(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        if cfg!(target_os = "macos")
            && self.view == View::Grid
            && self.focused.is_none()
            && widgets::icon_button(ui, Icon::Sidebar, "Toggle sidebar", 30.0, true).clicked()
        {
            actions.push(Action::ToggleSidebar);
        }
        let menu = widgets::icon_button(ui, Icon::Menu, "Main menu", 30.0, false);
        egui::Popup::menu(&menu).show(|ui| {
            ui.set_min_width(190.0);
            let showing = if ui.ctx().content_rect().width() < sidebar::SIDEBAR_MIN {
                self.sidebar_overlay
            } else {
                self.settings.sidebar
            };
            let sidebar = if showing {
                "Hide sidebar"
            } else {
                "Show sidebar"
            };
            for (label, action) in [
                (sidebar, Action::ToggleSidebar),
                ("Set SDK folder…", Action::Dialog(Dialog::Sdk)),
                ("Keyboard shortcuts", Action::Dialog(Dialog::Shortcuts)),
                ("About emugrid", Action::Dialog(Dialog::About)),
            ] {
                if ui.button(label).clicked() {
                    actions.push(action);
                    ui.close();
                }
            }
        });
    }

    pub(super) fn status_bar(&self, ui: &mut egui::Ui) {
        let p = widgets::pal(ui);
        egui::Panel::bottom("status")
            .exact_size(30.0)
            .frame(
                egui::Frame::new()
                    .fill(p.chrome)
                    .stroke(Stroke::new(1.0, p.border))
                    .inner_margin(egui::Margin::symmetric(12, 0)),
            )
            .show(ui, |ui| {
                let width = ui.available_width();
                ui.allocate_ui_with_layout(
                    Vec2::new(width, 28.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        let (dot, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                        let has_adb = self.sdk.adb.is_some();
                        widgets::paint_dot(
                            ui.painter(),
                            dot.center(),
                            if has_adb { p.live } else { p.polled },
                            false,
                        );
                        let root = self
                            .sdk
                            .root
                            .as_deref()
                            .map(|path| path.to_string_lossy())
                            .unwrap_or_else(|| {
                                if has_adb {
                                    "adb on PATH".into()
                                } else {
                                    "SDK missing".into()
                                }
                            });
                        let root_width = if self.message.is_some() { 0.25 } else { 0.32 };
                        status_label(
                            ui,
                            root.as_ref(),
                            FontId::monospace(10.5),
                            width * root_width,
                        );
                        ui.label(
                            RichText::new(format!(
                                "{} devices · {} running",
                                self.devices.len(),
                                self.running().count()
                            ))
                            .size(11.0)
                            .color(p.text3),
                        );
                        if let Some(message) = &self.message {
                            status_label(ui, message, FontId::proportional(11.0), width * 0.30);
                        }
                        if ui.available_width() >= 200.0 {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let hint = match (self.view, &self.focused) {
                                        (_, Some(_)) => "Esc to return to Grid",
                                        (View::Grid, _) => {
                                            "Click a tile to open · 1–9 jump to device"
                                        }
                                        (View::List, _) => {
                                            "Refreshes every 3 s · Drop an .apk anywhere"
                                        }
                                    };
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(hint).size(10.5).color(p.text3),
                                        )
                                        .truncate(),
                                    );
                                },
                            );
                        }
                    },
                );
            });
    }

    pub(super) fn show_dialog(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let Some(dialog) = self.dialog.clone() else {
            return;
        };
        let response = egui::Modal::new(egui::Id::unique("app-dialog")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 52.0).clamp(260.0, 410.0));
            let title = match dialog {
                Dialog::Sdk => "Android SDK folder",
                Dialog::Shortcuts => "Keyboard shortcuts",
                Dialog::About => "About emugrid",
                Dialog::Wipe { .. } => "Wipe emulator data?",
            };
            ui.heading(title);
            match &dialog {
                Dialog::Sdk => self.sdk_folder_controls(ui, actions),
                Dialog::Shortcuts => {
                    egui::ScrollArea::vertical()
                        .max_height((ctx.content_rect().height() - 130.0).max(80.0))
                        .show(ui, |ui| {
                            for shortcut in shortcuts::SHORTCUTS {
                                ui.horizontal(|ui| {
                                    widgets::kbd(ui, &shortcut.label());
                                    ui.label(shortcut.name);
                                });
                            }
                            for (key, name) in [
                                ("1–9".into(), "Open a visible device in Grid"),
                                (
                                    format!("{}–9", shortcuts::label(KeyCommand::Device(0))),
                                    "Switch focused device",
                                ),
                                ("Esc".into(), "Clear filter or return to Grid"),
                            ] {
                                ui.horizontal(|ui| {
                                    widgets::kbd(ui, &key);
                                    ui.label(name);
                                });
                            }
                        });
                }
                Dialog::About => {
                    ui.label(format!("emugrid {}", env!("CARGO_PKG_VERSION")));
                    ui.label("A desktop dashboard for Android devices and iOS simulators.");
                }
                Dialog::Wipe { avd, name } => {
                    ui.add(
                        egui::Label::new(format!(
                            "This permanently erases apps, files and settings for {name} ({avd})."
                        ))
                        .wrap(),
                    );
                    ui.horizontal_wrapped(|ui| {
                        if widgets::button(ui, Kind::Secondary, None, "Cancel").clicked() {
                            ui.close();
                        }
                        if widgets::button(ui, Kind::Danger, None, "Wipe data and start").clicked()
                        {
                            actions.push(Action::Start(avd.clone(), Boot::Wipe));
                            ui.close();
                        }
                    });
                    return;
                }
            }
            if widgets::button(ui, Kind::Secondary, None, "Done").clicked() {
                ui.close();
            }
        });
        if response.should_close() {
            self.dialog = None;
            self.sdk_path = None;
        }
    }

    pub(super) fn android_count(&self) -> usize {
        self.running()
            .filter(|d| d.platform == Platform::Android)
            .count()
    }
}

fn status_label(ui: &mut egui::Ui, text: &str, font: FontId, max_width: f32) {
    let color = widgets::pal(ui).text2;
    let galley = widgets::truncated(ui.painter(), text.to_owned(), font, color, max_width);
    let (rect, response) = ui.allocate_exact_size(galley.size(), Sense::hover());
    ui.painter().galley(rect.min, galley, color);
    response.on_hover_text(text);
}
