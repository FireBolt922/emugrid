use super::*;

impl EmuGrid {
    pub(super) fn grid_view(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let all: Vec<Device> = self
            .devices
            .iter()
            .filter(|d| matches!(d.state, State::Running { .. } | State::Booting { .. }))
            .cloned()
            .collect();
        if ui.ctx().content_rect().width() < toolbar::WIDE_TOOLBAR {
            self.filter_field(ui, "Filter devices", 150.0);
        }
        if all.is_empty() {
            self.empty_grid(ui, actions);
            return;
        }
        let android = all
            .iter()
            .filter(|d| d.platform == Platform::Android)
            .count();
        let ios = all.len() - android;
        if android == 0 || ios == 0 {
            self.grid_platform = None;
        }
        let running: Vec<_> = all
            .iter()
            .filter(|d| {
                self.grid_platform
                    .is_none_or(|platform| platform == d.platform)
                    && matches_filter(d, &self.filter)
            })
            .collect();
        let ready = running.iter().filter(|d| d.serial().is_some()).count();
        let visible = running
            .iter()
            .filter_map(|d| d.serial())
            .filter(|serial| self.last_grid_visible.contains(*serial))
            .count();
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Running devices").font(theme::semibold(17.0)));
            ui.label(
                RichText::new(format!(
                    "{visible} streaming · {} paused off-screen",
                    ready - visible
                ))
                .size(11.0)
                .color(widgets::pal(ui).text3),
            );
        });
        if android > 0 && ios > 0 {
            ui.horizontal_wrapped(|ui| {
                for (platform, label, count) in [
                    (None, "All", all.len()),
                    (Some(Platform::Android), "Android", android),
                    (Some(Platform::Ios), "iOS", ios),
                ] {
                    if ui
                        .selectable_label(
                            self.grid_platform == platform,
                            format!("{label} {count}"),
                        )
                        .clicked()
                    {
                        self.grid_platform = platform;
                        ui.ctx().request_repaint();
                    }
                }
            });
        }
        if running.is_empty() {
            ui.label("No devices match the filter");
            return;
        }
        let width_target = self.settings.tile_width as f32;
        let mut jump = Vec::new();
        let ppp = ui.ctx().pixels_per_point();
        egui::ScrollArea::vertical().show(ui, |ui| {
            let width = ui.available_width();
            let cols = grid::columns(width, width_target, GAP, running.len());
            let tile_w = ((width - GAP * (cols - 1) as f32) / cols as f32).min(width_target * 1.16);
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
                        if let Some(serial) = d.serial().filter(|s| self.visible.contains(*s)) {
                            jump.push(serial);
                        }
                    }
                });
                ui.add_space(GAP);
            }
        });
        let key = ui.input(|input| {
            input.events.iter().find_map(|event| {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } = event
                    && modifiers.is_none()
                {
                    return shortcuts::digit(*key);
                }
                None
            })
        });
        if !self.editing_text
            && !ui.ctx().egui_wants_keyboard_input()
            && !ui.ctx().any_popup_open()
            && self.dialog.is_none()
            && !self.sidebar_overlay
            && let Some(serial) = key.and_then(|index| jump.get(index))
        {
            actions.push(Action::Focus((*serial).into()));
        }
    }

    fn tile(&mut self, ui: &mut egui::Ui, d: &Device, size: Vec2, ppp: f32) -> Option<Action> {
        let serial = d.serial();
        let (rect, resp) = ui.allocate_exact_size(
            size,
            if serial.is_some() {
                Sense::click()
            } else {
                Sense::hover()
            },
        );
        if !ui.is_rect_visible(rect) {
            return None;
        }
        if let Some(serial) = serial {
            self.visible.insert(serial.into());
        }
        let hovered = serial.is_some() && resp.contains_pointer();
        let p = widgets::pal(ui);
        let painter = ui.painter_at(rect.expand(3.0));
        if hovered {
            painter.rect_stroke(
                rect.expand(1.5),
                RADIUS_CARD + 2,
                Stroke::new(3.0, p.accent_soft),
                StrokeKind::Outside,
            );
        }
        let (edge, width) = if hovered {
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
        let control = serial.and_then(|s| self.capture.control(s));
        let (label, tone) = self.status_badge(d);
        let pill = widgets::pill_size(&painter, &label);
        let name = widgets::truncated(
            &painter,
            self.label(d),
            theme::semibold(13.0),
            p.text,
            inner.width(),
        );
        painter.galley(inner.left_top(), name, p.text);
        let meta_y = inner.top() + 19.0;
        widgets::paint_pill(
            &painter,
            p,
            Rect::from_min_size(Pos2::new(inner.right() - pill.x, meta_y), pill),
            &label,
            tone,
        );
        let kind = widgets::truncated(
            &painter,
            device_kind(d),
            FontId::proportional(11.0),
            p.text3,
            (inner.width() - pill.x - 5.0).max(0.0),
        );
        painter.galley(Pos2::new(inner.left(), meta_y + 3.0), kind, p.text3);

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
        let img = serial.and_then(|s| self.paint_screen(&painter, s, screen_box, radius));
        if img.is_none() {
            let center = screen_box.center();
            egui::Spinner::new().size(22.0).color(p.text3).paint_at(
                ui,
                Rect::from_center_size(center - Vec2::new(0.0, 20.0), Vec2::splat(22.0)),
            );
            if matches!(d.state, State::Booting { .. }) {
                painter.text(
                    center,
                    egui::Align2::CENTER_CENTER,
                    "Booting…",
                    theme::semibold(13.0),
                    Color32::WHITE,
                );
                if let Some((start, boot)) = d.avd.as_ref().and_then(|avd| self.starts.get(avd)) {
                    let seconds = start.elapsed().as_secs();
                    let kind = match boot {
                        Boot::Normal => "quick boot",
                        Boot::Cold => "cold boot",
                        Boot::Wipe => "wipe and boot",
                    };
                    painter.text(
                        center + Vec2::new(0.0, 19.0),
                        egui::Align2::CENTER_CENTER,
                        format!("{kind} · {}:{:02}", seconds / 60, seconds % 60),
                        FontId::proportional(11.0),
                        Color32::LIGHT_GRAY,
                    );
                }
            }
        }

        // Footer: serial on the left, how the screen arrives on the right.
        let mid = inner.bottom() - TILE_FOOTER / 2.0;
        let transport = self.transport(d);
        let tag = widgets::tag_size(&painter, transport);
        widgets::paint_tag(
            &painter,
            p,
            Rect::from_min_size(Pos2::new(inner.right() - tag.x, mid - tag.y / 2.0), tag),
            transport,
        );
        let id = widgets::truncated(
            &painter,
            d.known_serial().or(d.avd.as_deref()).unwrap_or("").into(),
            FontId::monospace(11.0),
            p.text3,
            (inner.width() - tag.x - 8.0).max(20.0),
        );
        painter.galley(
            Pos2::new(inner.left(), mid - id.size().y / 2.0),
            id,
            p.text3,
        );

        let serial = serial?;
        if hovered {
            let bar_width = if d.platform == Platform::Android {
                4.0 * 25.0 + 10.0
            } else {
                35.0
            };
            let anchor = img.unwrap_or(screen_box);
            let bar = Rect::from_center_size(
                Pos2::new(anchor.center().x, anchor.bottom() - 21.0),
                Vec2::new(bar_width, 34.0),
            );
            let mut quick_action = None;
            let mut quick = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(("quick", serial))
                    .max_rect(bar)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            quick.set_clip_rect(ui.clip_rect().intersect(rect));
            quick.painter().rect_filled(bar, 8, p.popup);
            quick.add_space(4.0);
            quick.spacing_mut().item_spacing.x = 0.0;
            for (icon, label, command) in [
                (Icon::Back, "Back", KeyCommand::Back),
                (Icon::Home, "Home", KeyCommand::Home),
                (Icon::Recents, "Recents", KeyCommand::Recents),
                (Icon::Camera, "Screenshot", KeyCommand::Screenshot),
            ] {
                if (d.platform == Platform::Android || command == KeyCommand::Screenshot)
                    && widgets::icon_button(&mut quick, icon, label, 25.0, false).clicked()
                {
                    quick_action = self.device_shortcut(serial, command);
                }
            }
            if quick_action.is_some() || ui.rect_contains_pointer(bar) {
                return quick_action;
            }
        }
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(self.label(d))
            .clicked()
            .then(|| Action::Focus(serial.into()))
    }
}
