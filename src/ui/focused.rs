use super::*;

#[derive(Default, Clone, Copy, PartialEq)]
pub(super) enum Zoom {
    #[default]
    Fit,
    Actual,
    PixelPerfect,
}

impl Zoom {
    pub(super) fn texture_options(self) -> egui::TextureOptions {
        if self == Self::Fit {
            egui::TextureOptions::LINEAR
        } else {
            egui::TextureOptions::NEAREST
        }
    }
}

const RIPPLE_LIFETIME: Duration = Duration::from_millis(350);
const INSPECTOR_MIN: f32 = 900.0;

impl EmuGrid {
    pub(super) fn focused_view(
        &mut self,
        ui: &mut egui::Ui,
        serial: &str,
        actions: &mut Vec<Action>,
    ) {
        let Some(d) = self.device(serial).cloned() else {
            actions.push(Action::Unfocus);
            return;
        };
        let android = d.platform == Platform::Android;
        if android {
            self.ensure_screen_size(ui.ctx(), serial);
        }
        let width = ui.available_width();
        let expanded = width >= INSPECTOR_MIN;
        if expanded {
            self.details_open = false;
        }
        self.focused_header(ui, &d, serial, width, expanded, actions);
        if width >= 640.0 {
            egui::Panel::left("filmstrip")
                .exact_size(104.0)
                .frame(focused_panel(widgets::pal(ui)))
                .show(ui, |ui| self.filmstrip(ui, serial, actions));
        }
        if expanded {
            egui::Panel::right("inspector")
                .exact_size(294.0)
                .frame(focused_panel(widgets::pal(ui)))
                .show(ui, |ui| self.inspector(ui, &d, serial, actions));
        }
        self.focused_stage(ui, &d, serial, actions);
        if !expanded && self.details_open {
            let response =
                egui::Modal::new(egui::Id::unique("focused-details")).show(ui.ctx(), |ui| {
                    ui.set_width((width - 26.0).clamp(248.0, 294.0));
                    ui.set_height((ui.ctx().content_rect().height() - 120.0).clamp(110.0, 465.0));
                    ui.horizontal(|ui| {
                        ui.heading("Details");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::icon_button(ui, Icon::Close, "Close details", 28.0, false)
                                .clicked()
                            {
                                ui.close();
                            }
                        });
                    });
                    self.inspector(ui, &d, serial, actions);
                });
            if response.should_close() {
                self.details_open = false;
            }
        }
    }

    fn focused_header(
        &mut self,
        ui: &mut egui::Ui,
        d: &Device,
        serial: &str,
        width: f32,
        expanded: bool,
        actions: &mut Vec<Action>,
    ) {
        let (badge, tone) = self.status_badge(d);
        let p = widgets::pal(ui);
        ui.horizontal(|ui| {
            if widgets::button(ui, Kind::Ghost, Some(Icon::ChevronLeft), "Grid")
                .on_hover_text("Esc")
                .clicked()
            {
                actions.push(Action::Unfocus);
            }
            if width > 500.0 {
                ui.label(RichText::new("/").color(p.text3));
            }
            let switch_width = if width < 520.0 {
                (width - 213.0).clamp(68.0, 132.0)
            } else {
                165.0
            };
            egui::ComboBox::from_id_salt("device-switch")
                .width(switch_width)
                .truncate()
                .selected_text(RichText::new(self.label(d)).font(theme::semibold(13.0)))
                .show_ui(ui, |ui| {
                    for (index, other) in self.running().enumerate() {
                        let Some(next) = other.serial() else { continue };
                        let response = ui.selectable_label(next == serial, self.label(other));
                        let response = if index < 9 {
                            response.on_hover_text(shortcuts::label(KeyCommand::Device(index)))
                        } else {
                            response
                        };
                        if response.clicked() && next != serial {
                            actions.push(Action::Focus(next.into()));
                        }
                    }
                })
                .response
                .on_hover_text("Switch device");
            if width >= 520.0 {
                widgets::status_pill(ui, &badge, tone);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                control_button(
                    ui,
                    self,
                    serial,
                    (Icon::Camera, "Screenshot", KeyCommand::Screenshot),
                    actions,
                );
                if !expanded && widgets::button(ui, Kind::Secondary, None, "Details").clicked() {
                    self.lift_finger();
                    self.details_open = !self.details_open;
                }
                if d.platform == Platform::Android && width >= 670.0 {
                    widgets::icon(ui, Icon::Keyboard, 14.0, p.accent_text);
                    widgets::status_pill(ui, "Keyboard → device · Esc to release", Tone::Busy);
                }
            });
        });
        if width < 520.0 || (d.platform == Platform::Android && width < 670.0) {
            ui.horizontal_wrapped(|ui| {
                if width < 520.0 {
                    widgets::status_pill(ui, &badge, tone);
                }
                if d.platform == Platform::Android {
                    widgets::status_pill(ui, "Keyboard → device · Esc to release", Tone::Busy);
                }
            });
        }
        ui.add_space(4.0);
    }

    fn filmstrip(&self, ui: &mut egui::Ui, serial: &str, actions: &mut Vec<Action>) {
        let p = widgets::pal(ui);
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (index, d) in self.running().enumerate() {
                let Some(next) = d.serial() else { continue };
                ui.push_id(next, |ui| {
                    let (rect, response) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), 104.0),
                        Sense::click(),
                    );
                    if !ui.is_rect_visible(rect) {
                        return;
                    }
                    let active = serial == next;
                    let paint = ui.painter_at(rect);
                    paint.rect(
                        rect,
                        8,
                        if response.hovered() {
                            p.raised
                        } else {
                            p.surface
                        },
                        Stroke::new(
                            if active { 1.4 } else { 1.0 },
                            if active { p.accent } else { p.border },
                        ),
                        StrokeKind::Inside,
                    );
                    let frame = Rect::from_center_size(
                        Pos2::new(rect.center().x, rect.top() + 36.0),
                        Vec2::new(28.0, 57.0),
                    );
                    let (inner, radius) = widgets::bezel(&paint, p, frame, 6.0);
                    self.paint_screen(&paint, next, inner, radius);
                    let name = widgets::truncated(
                        &paint,
                        d.display_name.clone(),
                        theme::semibold(10.5),
                        p.text,
                        rect.width() - 8.0,
                    );
                    paint.galley(
                        Pos2::new(rect.center().x - name.size().x / 2.0, rect.top() + 69.0),
                        name,
                        p.text,
                    );
                    if index < 9 {
                        paint.text(
                            Pos2::new(rect.center().x, rect.top() + 86.0),
                            egui::Align2::CENTER_TOP,
                            shortcuts::label(KeyCommand::Device(index)),
                            FontId::monospace(9.5),
                            p.text3,
                        );
                    }
                    if response.on_hover_text(self.label(d)).clicked() && !active {
                        actions.push(Action::Focus(next.into()));
                    }
                });
            }
        });
    }

    fn inspector(
        &mut self,
        ui: &mut egui::Ui,
        d: &Device,
        serial: &str,
        actions: &mut Vec<Action>,
    ) {
        let p = widgets::pal(ui);
        let native = self.focused_native_size(serial);
        let height = ui.available_height();
        let action = if let Some(udid) = &d.udid {
            Some(("Shut down", Action::IosShutdown(udid.clone())))
        } else if d.is_emulator {
            Some(("Stop emulator", Action::Stop(serial.into())))
        } else {
            None
        };
        let footer = if action.is_some() { 38.0 } else { 0.0 };
        egui::ScrollArea::vertical()
            .id_salt("inspector-scroll")
            .max_height((height - footer).max(48.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.spacing_mut().interact_size.y = 22.0;
                widgets::section_label(ui, "Device");
                for (label, value, mono) in [
                    ("Model", d.display_name.clone(), false),
                    ("System", d.os.clone().unwrap_or_else(|| "—".into()), false),
                    (
                        "Screen",
                        native.map_or_else(|| "—".into(), dimensions),
                        false,
                    ),
                    ("Serial", serial.into(), true),
                    ("AVD", d.avd.as_deref().unwrap_or("—").into(), true),
                ] {
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            [49.0, 18.0],
                            egui::Label::new(RichText::new(label).size(11.0).color(p.text3)),
                        );
                        let value = if mono {
                            RichText::new(value).monospace()
                        } else {
                            RichText::new(value)
                        };
                        ui.add(egui::Label::new(value.size(11.5)).wrap());
                    });
                }
                ui.separator();
                widgets::section_label(ui, "Stream");
                let fps = self
                    .fps(serial)
                    .map_or_else(|| "—".into(), |value| value.to_string());
                let bucket = self.capture.control(serial).map_or_else(
                    || "—".into(),
                    |c| {
                        let px = c.size.load(Ordering::Relaxed);
                        if px == crate::capture::NATIVE_SIZE {
                            "Native".into()
                        } else {
                            format!("{px} px")
                        }
                    },
                );
                ui.columns(3, |columns| {
                    for (ui, (label, value)) in columns.iter_mut().zip([
                        ("FPS", fps),
                        ("SOURCE", self.transport(d).into()),
                        ("BUCKET", bucket),
                    ]) {
                        egui::Frame::new()
                            .fill(p.raised)
                            .corner_radius(8)
                            .inner_margin(6)
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.label(RichText::new(label).size(9.0).color(p.text3));
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&value).font(theme::semibold(11.0)),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(value);
                            });
                    }
                });
                if d.platform == Platform::Android {
                    ui.separator();
                    widgets::section_label(ui, "Send text");
                    ui.horizontal(|ui| {
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut self.send_text)
                                .hint_text("Text to send")
                                .desired_width((ui.available_width() - 69.0).max(60.0)),
                        );
                        let enter =
                            response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let send = widgets::button(ui, Kind::Secondary, None, "Send").clicked();
                        if (send || enter) && !self.send_text.is_empty() {
                            actions.push(Action::Shell(
                                serial.into(),
                                format!("input text {}", input::escape_text(&self.send_text)),
                            ));
                            self.send_text.clear();
                            if enter {
                                response.request_focus();
                            }
                        }
                    });
                    ui.small("Or just type with the device focused");
                    ui.separator();
                    widgets::section_label(ui, "Gestures");
                    for (key, help) in [
                        ("Click", "Tap"),
                        ("Drag", "Swipe"),
                        ("Hold, then drag", "Long press and move"),
                        ("Right-click", "Long press"),
                        ("Scroll", "Swipe (pan at 100%)"),
                        ("Esc", "Return to Grid"),
                    ] {
                        ui.horizontal(|ui| {
                            widgets::kbd(ui, key);
                            ui.label(RichText::new(help).size(11.0).color(p.text2));
                        });
                    }
                } else if let Some(udid) = &d.udid {
                    ui.separator();
                    widgets::status_pill(ui, "VIEW ONLY", Tone::Polled);
                    if widgets::button(ui, Kind::Secondary, None, "Open in Simulator").clicked() {
                        actions.push(Action::IosOpen(udid.clone()));
                    }
                }
            });
        if let Some((label, action)) = action
            && widgets::button(ui, Kind::Danger, Some(Icon::Stop), label).clicked()
        {
            actions.push(action);
        }
    }

    fn focused_native_size(&self, serial: &str) -> Option<(u32, u32)> {
        let native = self.capture.control(serial).and_then(|c| c.native_size());
        let known = native.or_else(|| self.screen_sizes.get(serial).copied());
        if let (Some((w, h)), Some(screen)) = (known, self.screens.get(serial))
            && (w > h) != (screen.size[0] > screen.size[1])
        {
            return Some((h, w));
        }
        known
    }

    fn focused_stage(
        &mut self,
        ui: &mut egui::Ui,
        d: &Device,
        serial: &str,
        actions: &mut Vec<Action>,
    ) {
        let p = widgets::pal(ui);
        let android = d.platform == Platform::Android;
        let available = ui.available_rect_before_wrap();
        let compact = available.width() < 400.0;
        let footer_height = if compact { 56.0 } else { 38.0 };
        let stage_size = Vec2::new(
            available.width().max(1.0),
            (available.height() - footer_height).max(36.0),
        );
        let (stage, _) = ui.allocate_exact_size(stage_size, Sense::hover());
        let dot = self.stage_dots.get_or_insert_with(|| {
            let mut image = egui::ColorImage::filled([14, 14], Color32::TRANSPARENT);
            image[(7, 7)] = Color32::WHITE;
            ui.ctx()
                .load_texture("stage-dots", image, egui::TextureOptions::NEAREST_REPEAT)
        });
        let painter = ui.painter_at(stage);
        painter.rect_filled(stage, RADIUS_CARD, p.chrome);
        painter.add(
            egui::epaint::RectShape::filled(stage, RADIUS_CARD, p.border_strong).with_texture(
                dot.id(),
                Rect::from_min_max(Pos2::ZERO, (stage.size() / 14.0).to_pos2()),
            ),
        );
        let rail_w = if android { 54.0 } else { 0.0 };
        let phone_area = Rect::from_min_max(stage.min, stage.max - Vec2::new(rail_w, 0.0));
        let ppp = ui.ctx().pixels_per_point();
        let native = self.focused_native_size(serial);
        let frame_size = self
            .screens
            .get(serial)
            .map(|s| (s.size[0] as u32, s.size[1] as u32));
        let (w, h) = native.or(frame_size).unwrap_or((1080, 2400));
        let pixels = Vec2::new(w as f32, h as f32);
        let frame_radius = f32::from(theme::RADIUS_PHONE);
        let pad = widgets::bezel_pad(frame_radius);
        let inset = pad + 5.0;
        let room = (phone_area.size() - Vec2::splat(inset * 2.0)).max(Vec2::splat(1.0));
        let fit = (room.x / pixels.x).min(room.y / pixels.y);
        let actual = self.zoom == Zoom::Actual;
        let scale = match self.zoom {
            Zoom::Fit => fit,
            Zoom::Actual => 1.0 / ppp,
            Zoom::PixelPerfect => pixel_perfect_scale(pixels, room * ppp) / ppp,
        };
        let screen = pixels * scale;
        if let Some(control) = self.capture.control(serial) {
            control.request_size(if self.zoom == Zoom::Fit {
                grid::bucket(screen.x.max(screen.y) * ppp)
            } else {
                crate::capture::NATIVE_SIZE
            });
        }
        let mut phone = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("stage-phone")
                .max_rect(phone_area),
        );
        phone.set_clip_rect(ui.clip_rect().intersect(phone_area));
        let mut frame = Rect::from_center_size(phone_area.center(), screen).expand(pad);
        if actual {
            egui::ScrollArea::both()
                .id_salt(("actual-pixels", serial))
                .scroll_source(
                    egui::scroll_area::ScrollSource::SCROLL_BAR
                        | egui::scroll_area::ScrollSource::MOUSE_WHEEL,
                )
                .max_width(phone_area.width())
                .max_height(phone_area.height())
                .auto_shrink([false; 2])
                .show(&mut phone, |ui| {
                    let canvas = (screen + Vec2::splat(inset * 2.0))
                        .max(phone_area.size() - Vec2::splat(14.0));
                    let (rect, _) = ui.allocate_exact_size(canvas, Sense::hover());
                    frame = Rect::from_center_size(rect.center(), screen).expand(pad);
                    self.draw_device(ui, d, frame, actions);
                });
        } else {
            self.draw_device(&mut phone, d, frame, actions);
        }
        if android {
            let rail_width = 44.0;
            let x = (frame.right() + 12.0).min(stage.right() - rail_width - 5.0);
            let height = (stage.height() - 10.0).clamp(30.0, 322.0);
            let rect = Rect::from_min_size(
                Pos2::new(x, stage.center().y - height / 2.0),
                Vec2::new(rail_width, height),
            );
            self.control_rail(ui, d, serial, rect, actions);
        }
        let status = native.or(frame_size).map_or_else(
            || "Waiting for screen…".into(),
            |size| {
                format!(
                    "{} · shown at {:.0}%",
                    dimensions(size),
                    scale * ppp * 100.0
                )
            },
        );
        let status_label = RichText::new(&status).monospace().size(10.5).color(p.text3);
        if compact {
            ui.label(status_label.clone());
        }
        ui.horizontal(|ui| {
            if !compact {
                ui.label(status_label);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for (zoom, label) in [
                    (Zoom::PixelPerfect, "Pixel-perfect"),
                    (Zoom::Actual, "100%"),
                    (Zoom::Fit, "Fit"),
                ] {
                    let button = ui
                        .selectable_label(self.zoom == zoom, label)
                        .on_hover_text(&status);
                    if button.clicked() && self.zoom != zoom {
                        actions.push(Action::Zoom(zoom));
                    }
                }
            });
        });
    }

    fn control_rail(
        &self,
        ui: &mut egui::Ui,
        d: &Device,
        serial: &str,
        rect: Rect,
        actions: &mut Vec<Action>,
    ) {
        let p = widgets::pal(ui);
        let mut rail = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::top_down(egui::Align::Center))
                .id_salt("control-rail"),
        );
        rail.set_clip_rect(ui.clip_rect().intersect(rect));
        egui::Frame::new()
            .fill(p.surface)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(14)
            .inner_margin(4)
            .show(&mut rail, |ui| {
                egui::ScrollArea::vertical()
                    .max_height((rect.height() - 10.0).max(10.0))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 3.0;
                        for (index, control) in [
                            (Icon::Back, "Back", KeyCommand::Back),
                            (Icon::Home, "Home", KeyCommand::Home),
                            (Icon::Recents, "Recents", KeyCommand::Recents),
                            (Icon::VolumeDown, "Volume down", KeyCommand::VolumeDown),
                            (Icon::VolumeUp, "Volume up", KeyCommand::VolumeUp),
                            (Icon::Power, "Power", KeyCommand::Power),
                            (Icon::Rotate, "Rotate", KeyCommand::Rotate),
                            (Icon::Camera, "Screenshot", KeyCommand::Screenshot),
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            if matches!(index, 3 | 5 | 7) {
                                ui.separator();
                            }
                            if control.2 != KeyCommand::Rotate || d.is_emulator {
                                control_button(ui, self, serial, control, actions);
                            }
                        }
                    });
            });
    }

    fn draw_device(
        &mut self,
        ui: &mut egui::Ui,
        d: &Device,
        frame: Rect,
        actions: &mut Vec<Action>,
    ) {
        let Some(serial) = d.serial() else {
            return;
        };
        let frame_radius = f32::from(theme::RADIUS_PHONE);
        let ppp = ui.ctx().pixels_per_point();
        let android = d.platform == Platform::Android;
        let p = widgets::pal(ui);
        let paint = ui.painter_at(frame.expand(3.0));
        paint.rect_stroke(
            frame.expand(1.0),
            frame_radius as u8 + 1,
            Stroke::new(1.5, p.accent),
            StrokeKind::Outside,
        );
        let (inner, radius) = widgets::bezel(&paint, p, frame, frame_radius);
        let img = self.screens.get(serial).map(|s| {
            let rect = if self.zoom == Zoom::Fit {
                self.screen_rect(serial, inner).unwrap_or(inner)
            } else {
                let min = (inner.min.to_vec2() * ppp).round() / ppp;
                Rect::from_min_size(min.to_pos2(), inner.size())
            };
            widgets::paint_rounded_texture(&paint, s.tex.id(), rect, radius);
            rect
        });
        if img.is_none() {
            egui::Spinner::new().size(24.0).color(p.text3).paint_at(
                ui,
                Rect::from_center_size(inner.center(), Vec2::splat(24.0)),
            );
        }
        let (Some(img), true) = (img, android) else {
            return;
        };
        let resp = ui.interact(
            img,
            ui.make_persistent_id("device-screen"),
            Sense::click_and_drag(),
        );
        self.stage_input(ui, serial, img, &resp, self.zoom != Zoom::Actual, actions);
        if let Some((uv, begun)) = self.ripple {
            if let Some(progress) = ripple_progress(begun.elapsed()) {
                let at = img.min + uv * img.size();
                let color = p.accent.gamma_multiply(1.0 - progress);
                let painter = paint.with_clip_rect(img.intersect(ui.clip_rect()));
                painter.circle_stroke(at, 5.0 + 21.0 * progress, Stroke::new(2.0, color));
                painter.circle_filled(at, 3.0, color);
                ui.ctx().request_repaint();
            } else {
                self.ripple = None;
            }
        }
    }

    fn stage_input(
        &mut self,
        ui: &egui::Ui,
        serial: &str,
        img: Rect,
        resp: &egui::Response,
        send_wheel: bool,
        actions: &mut Vec<Action>,
    ) {
        let shell = |cmd: &str| Action::Shell(serial.to_string(), cmd.to_string());
        let (Some(&screen), false) = (
            self.screen_sizes.get(serial),
            self.details_open || self.dialog.is_some(),
        ) else {
            return;
        };
        let landscape = self
            .screens
            .get(serial)
            .is_some_and(|s| s.size[0] > s.size[1]);
        let uv = |p: Pos2| {
            let uv = (p - img.min) / img.size();
            (uv.x, uv.y)
        };
        let to_dev = |p: Pos2| input::to_device(uv(p), screen, landscape);
        // A touch for the virtual touchscreen, with its `input` fallback.
        let touch = |steps: Vec<Touch>, fallback: String| {
            Action::Touch(serial.to_string(), steps, fallback)
        };
        let ctx = ui.ctx();
        let live_touch = self
            .input_workers
            .get(serial)
            .is_some_and(|w| w.live_touch());
        match self.gestures.update(resp, live_touch) {
            Some(input::Gesture::Tap(p)) if img.contains(p) => {
                self.ripple = Some(((p - img.min) / img.size(), Instant::now()));
                let (x, y) = to_dev(p);
                let (u, v) = uv(p);
                actions.push(touch(
                    vec![Touch::Down(u, v), Touch::Up],
                    format!("input tap {x} {y}"),
                ));
            }
            Some(input::Gesture::LongPress(p)) if img.contains(p) => {
                self.ripple = Some(((p - img.min) / img.size(), Instant::now()));
                let (x, y) = to_dev(p);
                actions.push(shell(&format!("input swipe {x} {y} {x} {y} 800")));
            }
            Some(input::Gesture::Swipe(a, b, ms)) if img.contains(a) => {
                let ((x1, y1), (x2, y2)) = (to_dev(a), to_dev(b));
                actions.push(shell(&format!("input swipe {x1} {y1} {x2} {y2} {ms}")));
            }
            Some(input::Gesture::Down(p)) if img.contains(p) => {
                self.ripple = Some(((p - img.min) / img.size(), Instant::now()));
                self.scroll = (0.0, None);
                let (x, y) = to_dev(p);
                self.finger = Some((serial.to_string(), (x, y)));
                let (u, v) = uv(p);
                actions.push(touch(
                    vec![Touch::Down(u, v)],
                    format!("input motionevent DOWN {x} {y}"),
                ));
            }
            Some(input::Gesture::Move(p)) if self.finger.is_some() => {
                let (x, y) = to_dev(p);
                self.finger = Some((serial.to_string(), (x, y)));
                let (u, v) = uv(p);
                actions.push(touch(
                    vec![Touch::Move(u, v)],
                    format!("input motionevent MOVE {x} {y}"),
                ));
            }
            Some(input::Gesture::Up(p)) if self.finger.is_some() => {
                let (x, y) = to_dev(p);
                if self.finger.take().is_some_and(|(_, last)| last != (x, y)) {
                    let (u, v) = uv(p);
                    actions.push(touch(
                        vec![Touch::Move(u, v)],
                        format!("input motionevent MOVE {x} {y}"),
                    ));
                }
                actions.push(touch(
                    vec![Touch::Up],
                    format!("input motionevent UP {x} {y}"),
                ));
            }
            Some(input::Gesture::Cancel) => self.lift_finger(),
            _ => {}
        }
        // Send the first wheel swipe immediately, then accumulate any deltas
        // that arrive before the next swipe can run.
        if send_wheel
            && self.finger.is_none()
            && resp.hovered()
            && ctx.input(|i| i.pointer.hover_pos().is_some_and(|p| img.contains(p)))
        {
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
            let limit = img.height() * 0.45;
            self.scroll.0 = (self.scroll.0 + delta).clamp(-limit, limit);
        }
        if send_wheel && self.scroll.0.abs() > 4.0 {
            let wait = self.scroll.1.map_or(Duration::ZERO, |t| {
                SCROLL_INTERVAL.saturating_sub(t.elapsed())
            });
            if wait.is_zero() {
                let center = img.center();
                let (x, y) = to_dev(center);
                let (_, end) = to_dev(center + Vec2::new(0.0, self.scroll.0));
                let (_, max) = to_dev(center + Vec2::new(0.0, img.height() * 0.45));
                self.scroll = (0.0, Some(Instant::now()));
                actions.push(Action::Scroll(
                    serial.into(),
                    input::adb::Command::Scroll {
                        x,
                        y,
                        distance: end as i32 - y as i32,
                        limit: (max - y) as i32,
                    },
                ));
            } else {
                ctx.request_repaint_after(wait);
            }
        }
    }

    /// Keyboard -> device, while a device is focused.
    pub(super) fn forward_keys(
        &mut self,
        ctx: &egui::Context,
        serial: &str,
        actions: &mut Vec<Action>,
    ) {
        let tabs = std::mem::take(&mut self.pending_tabs);
        if self.editing_text
            || self.popup_has_keys
            || ctx.text_edit_focused()
            || ctx.any_popup_open()
        {
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
}

fn dimensions((w, h): (u32, u32)) -> String {
    format!("{w} × {h}")
}

fn focused_panel(p: &theme::Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(p.chrome)
        .corner_radius(RADIUS_CARD)
        .inner_margin(8)
}

fn control_button(
    ui: &mut egui::Ui,
    app: &EmuGrid,
    serial: &str,
    (icon, label, command): (Icon, &str, KeyCommand),
    actions: &mut Vec<Action>,
) {
    let tip = format!("{label} · {}", shortcuts::label(command));
    if widgets::icon_button(ui, icon, &tip, 32.0, false).clicked()
        && let Some(action) = app.device_shortcut(serial, command)
    {
        actions.push(action);
    }
}

fn pixel_perfect_scale(pixels: Vec2, room: Vec2) -> f32 {
    1.0 / (pixels.x / room.x).max(pixels.y / room.y).ceil().max(1.0)
}

fn ripple_progress(elapsed: Duration) -> Option<f32> {
    (elapsed < RIPPLE_LIFETIME).then(|| elapsed.as_secs_f32() / RIPPLE_LIFETIME.as_secs_f32())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_perfect_uses_a_whole_divisor_at_physical_scale() {
        let phone = Vec2::new(1080.0, 2400.0);
        assert_eq!(
            pixel_perfect_scale(phone, Vec2::new(900.0, 1000.0)),
            1.0 / 3.0
        );
        assert_eq!(pixel_perfect_scale(phone, Vec2::new(540.0, 1200.0)), 0.5);
        assert_eq!(pixel_perfect_scale(phone, phone * 2.0), 1.0);
    }

    #[test]
    fn tap_ripple_expires_at_350_ms() {
        assert_eq!(ripple_progress(Duration::ZERO), Some(0.0));
        assert_eq!(ripple_progress(Duration::from_millis(175)), Some(0.5));
        assert_eq!(ripple_progress(Duration::from_millis(350)), None);
    }
}
