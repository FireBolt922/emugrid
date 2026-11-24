//! Sending touches and keys to Android devices with `adb shell input`.

use eframe::egui::{Pos2, Response};

pub mod adb;
pub mod wda;

/// Quote text for `adb shell input text`: spaces become `%s`, and the whole
/// thing is single-quoted for the device shell.
pub fn escape_text(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''").replace(' ', "%s"))
}

/// Parse `adb shell wm size` (an override wins over the physical size).
pub fn parse_wm_size(out: &str) -> Option<(u32, u32)> {
    let size = |prefix: &str| {
        out.lines()
            .find_map(|l| l.trim().strip_prefix(prefix))
            .and_then(|s| s.trim().split_once('x'))
            .and_then(|(w, h)| Some((w.trim().parse().ok()?, h.trim().parse().ok()?)))
    };
    size("Override size:").or_else(|| size("Physical size:"))
}

/// Map a point in the shown image (0..1, 0..1) to device pixels. The screen
/// size from `wm size` is always portrait, so follow the frame's orientation.
pub fn to_device(uv: (f32, f32), screen: (u32, u32), frame_landscape: bool) -> (u32, u32) {
    let (short, long) = (screen.0.min(screen.1), screen.0.max(screen.1));
    let (w, h) = if frame_landscape {
        (long, short)
    } else {
        (short, long)
    };
    let axis =
        |t: f32, len: u32| ((t.clamp(0.0, 1.0) * len as f32) as u32).min(len.saturating_sub(1));
    (axis(uv.0, w), axis(uv.1, h))
}

/// Android key code for an egui key we forward.
pub fn keycode(key: eframe::egui::Key) -> Option<u32> {
    use eframe::egui::Key;
    Some(match key {
        Key::Backspace => 67,
        Key::Enter => 66,
        Key::Tab => 61,
        Key::Delete => 112,
        Key::ArrowUp => 19,
        Key::ArrowDown => 20,
        Key::ArrowLeft => 21,
        Key::ArrowRight => 22,
        _ => return None,
    })
}

/// Remove Tab key events (egui would use them to move widget focus) and
/// return how many Tab presses there were, so they can go to the device.
pub fn take_tabs(events: &mut Vec<eframe::egui::Event>) -> usize {
    use eframe::egui::{Event, Key};
    let presses = events
        .iter()
        .filter(|e| {
            matches!(
                e,
                Event::Key {
                    key: Key::Tab,
                    pressed: true,
                    ..
                }
            )
        })
        .count();
    events.retain(|e| !matches!(e, Event::Key { key: Key::Tab, .. }));
    presses
}

/// Pointer movement (points) below which a press counts as stationary.
const SWIPE_MIN_DISTANCE: f32 = 6.0;
/// Minimum duration of a long press on a device without live touch.
const HOLD_MS: u32 = 400;
/// Live finger: smallest pointer movement (points) worth sending.
const MOVE_MIN_DISTANCE: f32 = 2.0;

/// A pointer gesture on the device screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Gesture {
    Tap(Pos2),
    LongPress(Pos2),
    /// From, to, duration in ms.
    Swipe(Pos2, Pos2, u32),
    /// Live finger: touch down, move, lift.
    Down(Pos2),
    Move(Pos2),
    Up(Pos2),
    Cancel,
}

#[derive(Debug, Clone, Copy)]
struct Press {
    from: Pos2,
    t0: f64,
    button: eframe::egui::PointerButton,
    /// Live finger on the device, and where it was last sent.
    live: Option<Pos2>,
}

/// Turns egui pointer interaction on one widget into device gestures.
#[derive(Default)]
pub struct GestureTracker {
    press: Option<Press>,
}

impl GestureTracker {
    pub fn update(&mut self, resp: &Response, live_touch: bool) -> Option<Gesture> {
        use eframe::egui::PointerButton::{Primary, Secondary};

        // egui forgets the press origin on the pass that reports the release,
        // so remember where and when the press started ourselves.
        let (now, pos, origin, primary, secondary, focused) = resp.ctx.input(|i| {
            (
                i.time,
                i.pointer.interact_pos(),
                i.pointer.press_origin(),
                i.pointer.primary_down(),
                i.pointer.secondary_down(),
                i.focused,
            )
        });
        if !focused {
            return self
                .press
                .take()
                .and_then(|p| p.live)
                .map(|_| Gesture::Cancel);
        }
        if let Some(p) = self.press.as_mut() {
            let down = match p.button {
                Primary => primary,
                Secondary => secondary,
                _ => unreachable!(),
            };
            if down {
                if let (Some(last), Some(at)) = (p.live, pos)
                    && at.distance(last) >= MOVE_MIN_DISTANCE
                {
                    p.live = Some(at);
                    return Some(Gesture::Move(at));
                }
                return None;
            }
        } else if (primary || secondary) && resp.is_pointer_button_down_on() {
            let from = origin.or(pos)?;
            let live = (primary && live_touch).then_some(from);
            self.press = Some(Press {
                from,
                t0: now,
                button: if primary { Primary } else { Secondary },
                live,
            });
            return live.map(Gesture::Down);
        }
        // A quick tap (trackpad tap-to-click) can press and release within one
        // frame, so the press was never seen as "down": use egui's click.
        let Some(Press {
            from,
            t0,
            live,
            button,
        }) = self.press.take()
        else {
            let at = resp.interact_pointer_pos().or(pos)?;
            return if resp.secondary_clicked() {
                Some(Gesture::LongPress(at))
            } else if resp.clicked() {
                Some(Gesture::Tap(at))
            } else {
                None
            };
        };
        if let Some(last) = live {
            return Some(Gesture::Up(pos.unwrap_or(last)));
        }
        let to = pos.unwrap_or(from);
        let held = ((now - t0).max(0.0) * 1000.0).round() as u32;
        if button == Secondary {
            return resp.secondary_clicked().then_some(Gesture::LongPress(from));
        }
        Some(if from.distance(to) > SWIPE_MIN_DISTANCE {
            Gesture::Swipe(from, to, held.clamp(80, 2000))
        } else if held >= HOLD_MS {
            Gesture::LongPress(from)
        } else {
            Gesture::Tap(from)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::Key;

    #[test]
    fn escapes_spaces_and_quotes() {
        assert_eq!(escape_text("it's a b"), r"'it'\''s%sa%sb'");
        assert_eq!(escape_text("$HOME;&|"), "'$HOME;&|'");
    }

    #[test]
    fn parses_wm_size() {
        assert_eq!(
            parse_wm_size("Physical size: 1080x2400\n"),
            Some((1080, 2400))
        );
        assert_eq!(
            parse_wm_size("Physical size: 1080x2400\nOverride size: 720x1600\n"),
            Some((720, 1600))
        );
        assert_eq!(parse_wm_size("error"), None);
    }

    #[test]
    fn maps_coordinates_with_rotation() {
        assert_eq!(to_device((0.5, 0.5), (1080, 2400), false), (540, 1200));
        assert_eq!(to_device((1.0, 0.0), (1080, 2400), true), (2399, 0));
        assert_eq!(to_device((-0.2, 1.5), (1080, 2400), false), (0, 2399));
    }

    #[test]
    fn maps_keys() {
        assert_eq!(keycode(Key::Backspace), Some(67));
        assert_eq!(keycode(Key::Enter), Some(66));
        assert_eq!(keycode(Key::A), None);
    }

    #[test]
    fn takes_tab_presses_out_of_egui_input() {
        use eframe::egui::{Event, Modifiers};
        let key = |key, pressed| Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        let mut events = vec![
            Event::Text("a".into()),
            key(Key::Tab, true),
            key(Key::Tab, false),
            key(Key::Enter, true),
        ];
        assert_eq!(take_tabs(&mut events), 1);
        assert_eq!(events, vec![Event::Text("a".into()), key(Key::Enter, true)]);
    }

    mod gestures {
        use super::super::*;
        use PointerButton::{Primary, Secondary};
        use eframe::egui::{self, Event, PointerButton, RawInput, Rect, Sense, Vec2};

        /// Feed pointer events through real egui passes, one pass per step.
        fn run(steps: &[(f64, Vec<Event>)], live_touch: bool) -> Vec<Gesture> {
            let ctx = egui::Context::default();
            let mut tracker = GestureTracker::default();
            let mut out = Vec::new();
            for (t, events) in steps {
                let raw = RawInput {
                    time: Some(*t),
                    events: events.clone(),
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0))),
                    ..Default::default()
                };
                let mut output = ctx.run_ui(raw, |ui| {
                    let (_, resp) =
                        ui.allocate_exact_size(Vec2::splat(380.0), Sense::click_and_drag());
                    out.extend(tracker.update(&resp, live_touch));
                });
                output.textures_delta.clear();
            }
            out
        }

        fn mv(x: f32, y: f32) -> Event {
            Event::PointerMoved(Pos2::new(x, y))
        }

        fn button(x: f32, y: f32, button: PointerButton, pressed: bool) -> Event {
            Event::PointerButton {
                pos: Pos2::new(x, y),
                button,
                pressed,
                modifiers: Default::default(),
            }
        }

        #[test]
        fn drag_is_a_swipe_from_press_to_release() {
            let g = run(
                &[
                    (0.0, vec![mv(100.0, 100.0)]),
                    (0.1, vec![button(100.0, 100.0, Primary, true)]),
                    (0.2, vec![mv(150.0, 160.0)]),
                    (0.3, vec![mv(200.0, 250.0)]),
                    (0.5, vec![button(200.0, 250.0, Primary, false)]),
                    (0.6, vec![]),
                ],
                false,
            );
            assert_eq!(
                g,
                vec![Gesture::Swipe(
                    Pos2::new(100.0, 100.0),
                    Pos2::new(200.0, 250.0),
                    400
                )]
            );
        }

        #[test]
        fn quick_click_is_a_tap() {
            let g = run(
                &[
                    (0.0, vec![mv(50.0, 60.0)]),
                    (0.1, vec![button(50.0, 60.0, Primary, true)]),
                    (0.2, vec![button(50.0, 60.0, Primary, false)]),
                    (0.3, vec![]),
                ],
                false,
            );
            assert_eq!(g, vec![Gesture::Tap(Pos2::new(50.0, 60.0))]);
        }

        /// Trackpad tap-to-click delivers press and release in one frame.
        #[test]
        fn press_and_release_in_one_frame_is_a_tap() {
            let g = run(
                &[
                    (0.0, vec![mv(50.0, 60.0)]),
                    (
                        0.1,
                        vec![
                            button(50.0, 60.0, Primary, true),
                            button(50.0, 60.0, Primary, false),
                        ],
                    ),
                    (0.2, vec![]),
                ],
                true,
            );
            assert_eq!(g, vec![Gesture::Tap(Pos2::new(50.0, 60.0))]);
        }

        /// Holding still is a real long press: down while held, up on release.
        #[test]
        fn holding_still_is_a_live_long_press() {
            let g = run(
                &[
                    (0.0, vec![mv(50.0, 60.0)]),
                    (0.1, vec![button(50.0, 60.0, Primary, true)]),
                    (0.6, vec![]),
                    (1.5, vec![button(50.0, 60.0, Primary, false)]),
                    (1.6, vec![]),
                ],
                true,
            );
            assert_eq!(
                g,
                vec![
                    Gesture::Down(Pos2::new(50.0, 60.0)),
                    Gesture::Up(Pos2::new(50.0, 60.0)),
                ]
            );
        }

        #[test]
        fn drag_moves_a_live_finger_before_release() {
            let g = run(
                &[
                    (0.0, vec![mv(50.0, 60.0)]),
                    (0.1, vec![button(50.0, 60.0, Primary, true)]),
                    (0.2, vec![mv(120.0, 60.0)]),
                    (0.3, vec![mv(200.0, 90.0)]),
                    (0.4, vec![button(200.0, 90.0, Primary, false)]),
                ],
                true,
            );
            assert_eq!(
                g,
                vec![
                    Gesture::Down(Pos2::new(50.0, 60.0)),
                    Gesture::Move(Pos2::new(120.0, 60.0)),
                    Gesture::Move(Pos2::new(200.0, 90.0)),
                    Gesture::Up(Pos2::new(200.0, 90.0)),
                ]
            );
        }

        #[test]
        fn right_click_is_a_long_press() {
            let g = run(
                &[
                    (0.0, vec![mv(70.0, 80.0)]),
                    (0.1, vec![button(70.0, 80.0, Secondary, true)]),
                    (0.2, vec![button(70.0, 80.0, Secondary, false)]),
                    (0.3, vec![]),
                ],
                true,
            );
            assert_eq!(g, vec![Gesture::LongPress(Pos2::new(70.0, 80.0))]);
        }
    }
}
