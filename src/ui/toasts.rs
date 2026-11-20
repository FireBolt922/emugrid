use super::*;

pub(super) enum Notice {
    Info,
    Success,
    Error,
}

#[derive(Default)]
pub(super) struct Toasts {
    next: u64,
    entries: Vec<Toast>,
}

struct Toast {
    id: u64,
    content: Content,
    expires: Option<Instant>,
}

enum Content {
    Notice(Notice, String),
    Install { apk: PathBuf, targets: Vec<Target> },
}

struct Target {
    serial: String,
    label: String,
    state: InstallState,
}

enum InstallState {
    Queued,
    Installing,
    Installed,
    Failed(devices::InstallFailure),
}

impl Toasts {
    fn add(&mut self, content: Content, expires: Option<Instant>) -> u64 {
        self.next += 1;
        self.entries.push(Toast {
            id: self.next,
            content,
            expires,
        });
        self.next
    }

    pub(super) fn push(&mut self, kind: Notice, text: impl Into<String>) {
        let expires = match kind {
            Notice::Error => None,
            Notice::Info | Notice::Success => Some(Instant::now() + Duration::from_secs(5)),
        };
        self.add(Content::Notice(kind, text.into()), expires);
    }

    pub(super) fn install(&mut self, apk: PathBuf, targets: Vec<(String, String)>) -> u64 {
        let targets = targets
            .into_iter()
            .map(|(serial, label)| Target {
                serial,
                label,
                state: InstallState::Queued,
            })
            .collect();
        self.add(Content::Install { apk, targets }, None)
    }

    pub(super) fn begin_install(&mut self, id: u64, serial: &str) {
        if let Some((_, targets)) = self.install_data(id)
            && let Some(target) = targets.iter_mut().find(|target| target.serial == serial)
        {
            target.state = InstallState::Installing;
        }
    }

    pub(super) fn finish_install(
        &mut self,
        id: u64,
        serial: &str,
        result: Result<(), devices::InstallFailure>,
    ) {
        let Some(toast) = self.entries.iter_mut().find(|toast| toast.id == id) else {
            return;
        };
        if let Content::Install { targets, .. } = &mut toast.content
            && let Some(target) = targets.iter_mut().find(|target| target.serial == serial)
        {
            target.state = match result {
                Ok(()) => InstallState::Installed,
                Err(error) => InstallState::Failed(error),
            };
            if targets
                .iter()
                .all(|target| matches!(target.state, InstallState::Installed))
            {
                toast.expires = Some(Instant::now() + Duration::from_secs(8));
            }
        }
    }

    pub(super) fn retry_install(&mut self, id: u64, serial: &str) -> Option<(PathBuf, bool)> {
        let (apk, targets) = self.install_data(id)?;
        let target = targets.iter_mut().find(|target| target.serial == serial)?;
        let InstallState::Failed(error) = &target.state else {
            return None;
        };
        let downgrade = error.allows_downgrade();
        target.state = InstallState::Queued;
        Some((apk.clone(), downgrade))
    }

    fn install_data(&mut self, id: u64) -> Option<(&PathBuf, &mut Vec<Target>)> {
        let toast = self.entries.iter_mut().find(|toast| toast.id == id)?;
        match &mut toast.content {
            Content::Install { apk, targets } => Some((apk, targets)),
            _ => None,
        }
    }

    pub(super) fn show(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let now = Instant::now();
        self.entries
            .retain(|t| t.expires.is_none_or(|end| end > now));
        if self.entries.is_empty() {
            return;
        }
        let width = (ctx.content_rect().width() - 28.0).clamp(160.0, 365.0);
        let height = (ctx.content_rect().height() * 0.55).max(100.0);
        egui::Area::new(egui::Id::unique("notifications"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_BOTTOM, [-14.0, -42.0])
            .default_size([width, height])
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.set_max_height(height);
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .max_height(height)
                    .show(ui, |ui| {
                        self.entries.retain(|toast| {
                            if let Some(end) = toast.expires {
                                ctx.request_repaint_after(end.saturating_duration_since(now));
                            }
                            !ui.push_id(toast.id, |ui| toast.show(ui, actions)).inner
                        });
                    });
            });
    }
}

impl Toast {
    fn show(&self, ui: &mut egui::Ui, actions: &mut Vec<Action>) -> bool {
        let p = widgets::pal(ui);
        egui::Frame::popup(ui.style())
            .inner_margin(10)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let (icon, color, text) = match &self.content {
                    Content::Notice(kind, text) => {
                        let (icon, color) = match kind {
                            Notice::Info => (Icon::Info, p.accent),
                            Notice::Success => (Icon::Check, p.live),
                            Notice::Error => (Icon::Warning, p.danger),
                        };
                        (icon, color, text.clone())
                    }
                    Content::Install { apk, targets } => {
                        let done = targets
                            .iter()
                            .filter(|target| {
                                matches!(
                                    target.state,
                                    InstallState::Installed | InstallState::Failed(_)
                                )
                            })
                            .count();
                        let name = apk
                            .file_name()
                            .map(|name| name.to_string_lossy())
                            .unwrap_or_default();
                        let heading = if done < targets.len() {
                            "Installing"
                        } else {
                            "Install results for"
                        };
                        (
                            Icon::Upload,
                            p.accent,
                            format!("{heading} {name} · {done} of {} done", targets.len()),
                        )
                    }
                };
                let close = ui
                    .horizontal_top(|ui| {
                        widgets::icon(ui, icon, 16.0, color);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                            let close = widgets::icon_button(
                                ui,
                                Icon::Close,
                                "Dismiss notification",
                                26.0,
                                false,
                            )
                            .clicked();
                            ui.add(egui::Label::new(text).wrap());
                            close
                        })
                        .inner
                    })
                    .inner;
                if let Content::Install { targets, .. } = &self.content {
                    for target in targets {
                        ui.push_id(&target.serial, |ui| target.show(ui, self.id, actions));
                    }
                }
                close
            })
            .inner
    }
}

impl Target {
    fn show(&self, ui: &mut egui::Ui, id: u64, actions: &mut Vec<Action>) {
        let p = widgets::pal(ui);
        ui.separator();
        ui.horizontal(|ui| {
            ui.add(
                egui::Label::new(RichText::new(&self.label).font(theme::semibold(11.5))).truncate(),
            )
            .on_hover_text(&self.serial);
        });
        match &self.state {
            InstallState::Queued => {
                ui.small("Queued");
            }
            InstallState::Installing => {
                ui.small("Installing…");
                let (bar, _) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 4.0), Sense::hover());
                let offset = ui.input(|i| (i.time * 0.9).fract() as f32) * bar.width() * 0.74;
                ui.painter().rect_filled(bar, 2, p.control);
                ui.painter().rect_filled(
                    Rect::from_min_size(
                        bar.min + Vec2::new(offset, 0.0),
                        Vec2::new(bar.width() * 0.26, bar.height()),
                    ),
                    2,
                    p.accent,
                );
                ui.ctx().request_repaint_after(Duration::from_millis(32));
            }
            InstallState::Installed => {
                ui.label(RichText::new("Installed").size(11.0).color(p.live));
            }
            InstallState::Failed(error) => {
                ui.horizontal_wrapped(|ui| {
                    let retry = if error.allows_downgrade() {
                        "Retry with -d"
                    } else {
                        "Retry"
                    };
                    if widgets::button(ui, Kind::Secondary, None, retry).clicked() {
                        actions.push(Action::RetryInstall(id, self.serial.clone()));
                    }
                    ui.label(RichText::new("Failed").size(11.0).color(p.danger));
                });
                let detail = error
                    .code
                    .as_deref()
                    .unwrap_or_else(|| error.detail.lines().next().unwrap_or("Unknown error"));
                ui.add(
                    egui::Label::new(RichText::new(detail).monospace().size(10.5).color(p.danger))
                        .truncate(),
                )
                .on_hover_text(&error.detail);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_target_can_be_retried_once_until_it_finishes() {
        let mut toasts = Toasts::default();
        let id = toasts.install("app.apk".into(), vec![("serial".into(), "Pixel 8".into())]);
        assert!(toasts.retry_install(id, "serial").is_none());
        toasts.begin_install(id, "serial");
        toasts.finish_install(
            id,
            "serial",
            Err(devices::InstallFailure {
                code: Some("INSTALL_FAILED_VERSION_DOWNGRADE".into()),
                detail: "older version".into(),
            }),
        );
        assert_eq!(
            toasts.retry_install(id, "serial"),
            Some(("app.apk".into(), true))
        );
        assert!(toasts.retry_install(id, "serial").is_none());
        toasts.begin_install(id, "serial");
        toasts.finish_install(id, "serial", Ok(()));
        assert!(toasts.entries[0].expires.is_some());
    }
}
