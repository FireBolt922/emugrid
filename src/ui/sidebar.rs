use super::*;

pub(super) const SIDEBAR_MIN: f32 = 690.0;

impl EmuGrid {
    pub(super) fn sidebar(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        if self.view != View::Grid || self.focused.is_some() {
            self.sidebar_overlay = false;
            return;
        }
        let width = ui.ctx().content_rect().width();
        if width >= SIDEBAR_MIN {
            self.sidebar_overlay = false;
            if self.settings.sidebar {
                let p = widgets::pal(ui);
                egui::Panel::left("device-sidebar")
                    .exact_size(236.0)
                    .frame(
                        egui::Frame::new()
                            .fill(p.chrome)
                            .stroke(Stroke::new(1.0, p.border))
                            .inner_margin(11),
                    )
                    .show(ui, |ui| self.sidebar_content(ui, actions));
            }
        } else if self.sidebar_overlay {
            let ctx = ui.ctx();
            let response = egui::Modal::new(egui::Id::unique("compact-sidebar")).show(ctx, |ui| {
                ui.set_width(235.0);
                ui.set_height((ctx.content_rect().height() - 78.0).clamp(180.0, 570.0));
                ui.horizontal(|ui| {
                    ui.heading("Devices");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::icon_button(ui, Icon::Close, "Close sidebar", 28.0, false)
                            .clicked()
                        {
                            ui.close();
                        }
                    });
                });
                self.sidebar_content(ui, actions);
            });
            if response.should_close() {
                self.sidebar_overlay = false;
            }
        }
    }

    fn sidebar_content(&self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let height = ui.available_height();
        egui::ScrollArea::vertical()
            .max_height((height - 70.0).max(62.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                widgets::section_label(ui, &format!("Running · {}", self.running().count()));
                for device in self.running() {
                    let Some(serial) = device.serial() else {
                        continue;
                    };
                    let (meta, tone) = self.status_badge(device);
                    if sidebar_row(ui, &device.display_name, &meta, tone) {
                        actions.push(Action::Focus(serial.into()));
                    }
                }
                ui.add_space(9.0);
                for (platform, title, button) in [
                    (Platform::Android, "Android · stopped", "Start"),
                    (Platform::Ios, "iOS · shut down", "Boot"),
                ] {
                    if platform == Platform::Ios && !cfg!(target_os = "macos") {
                        continue;
                    }
                    let stopped: Vec<_> = self
                        .devices
                        .iter()
                        .filter(|d| d.platform == platform && d.state == State::Stopped)
                        .collect();
                    widgets::section_label(ui, &format!("{title} · {}", stopped.len()));
                    for device in stopped {
                        ui.horizontal(|ui| {
                            let width = (ui.available_width() - 84.0).max(40.0);
                            ui.allocate_ui_with_layout(
                                Vec2::new(width, 30.0),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&device.display_name).size(11.5),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(device.title());
                                },
                            );
                            if widgets::button(ui, Kind::Ghost, Some(Icon::Play), button).clicked()
                            {
                                if let Some(avd) = &device.avd {
                                    actions.push(Action::Start(avd.clone(), Boot::Normal));
                                } else if let Some(udid) = &device.udid {
                                    actions.push(Action::IosBoot(udid.clone()));
                                }
                            }
                        });
                    }
                    ui.add_space(8.0);
                }
            });
        self.sidebar_drop_card(ui);
    }

    fn sidebar_drop_card(&self, ui: &mut egui::Ui) {
        let p = widgets::pal(ui);
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 60.0), Sense::hover());
        let rect = rect.shrink(1.0);
        let points = [
            rect.left_top(),
            rect.right_top(),
            rect.right_bottom(),
            rect.left_bottom(),
            rect.left_top(),
        ];
        ui.painter().extend(egui::Shape::dashed_line(
            &points,
            Stroke::new(1.0, p.border_strong),
            5.0,
            4.0,
        ));
        let n = self.android_count();
        let text = format!(
            "Drop an .apk anywhere to install on {n} Android {}",
            if n == 1 { "device" } else { "devices" }
        );
        let galley = ui.painter().layout(
            text,
            FontId::proportional(11.0),
            p.text2,
            (rect.width() - 22.0).max(20.0),
        );
        ui.painter()
            .galley(rect.center() - galley.size() / 2.0, galley, p.text2);
    }
}

fn sidebar_row(ui: &mut egui::Ui, label: &str, meta: &str, tone: Tone) -> bool {
    let p = widgets::pal(ui);
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 36.0), Sense::click());
    if !ui.is_rect_visible(rect) {
        return false;
    }
    let painter = ui.painter_at(rect);
    if response.hovered() {
        painter.rect_filled(rect, 8, p.raised);
    }
    widgets::paint_dot(
        &painter,
        rect.left_center() + Vec2::new(9.0, 0.0),
        tone.colors(p).0,
        false,
    );
    for (text, font, color, offset) in [
        (label, theme::semibold(11.5), p.text, 3.0),
        (meta, FontId::proportional(10.0), p.text3, 18.0),
    ] {
        let galley = widgets::truncated(
            &painter,
            text.to_owned(),
            font,
            color,
            (rect.width() - 25.0).max(10.0),
        );
        painter.galley(rect.left_top() + Vec2::new(20.0, offset), galley, color);
    }
    response.on_hover_text(label).clicked()
}
