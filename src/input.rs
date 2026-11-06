//! Sending touches and keys to Android devices with `adb shell input`.

use eframe::egui::{Pos2, Response};

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

/// Next `(serial, shell command)` to run. Pulls in everything already
/// queued, and skips a live-finger MOVE when a newer MOVE for the same device
/// is waiting, so a slow adb never makes the finger lag behind the pointer.
pub fn next_command(
    queue: &mut std::collections::VecDeque<(String, String)>,
    incoming: &std::sync::mpsc::Receiver<(String, String)>,
) -> Option<(String, String)> {
    queue.extend(incoming.try_iter());
    let is_move = |c: &str| c.starts_with("input motionevent MOVE ");
    loop {
        let item = queue.pop_front()?;
        let stale = is_move(&item.1)
            && queue
                .front()
                .is_some_and(|(s, c)| *s == item.0 && is_move(c));
        if !stale {
            return Some(item);
        }
    }
}

/// Pointer movement (points) below which a press counts as stationary.
const SWIPE_MIN_DISTANCE: f32 = 6.0;
/// Holding still this long puts a live finger on the device (long press,
/// then drag).
pub const HOLD_MS: u32 = 400;
/// Live finger: smallest pointer movement (points) worth sending.
const MOVE_MIN_DISTANCE: f32 = 2.0;

/// A pointer gesture on the device screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Gesture {
    Tap(Pos2),
    LongPress(Pos2),
    /// From, to, duration in ms.
    Swipe(Pos2, Pos2, u32),
    /// Live finger (after holding still): touch down, move, lift.
    Down(Pos2),
    Move(Pos2),
    Up(Pos2),
}

#[derive(Debug, Clone, Copy)]
struct Press {
    from: Pos2,
    t0: f64,
    /// Moved past the swipe threshold before the hold was recognised.
    moved: bool,
    /// Live finger on the device, and where it was last sent.
    live: Option<Pos2>,
}

/// Turns egui pointer interaction on one widget into device gestures.
#[derive(Default)]
pub struct GestureTracker {
    press: Option<Press>,
}

impl GestureTracker {
    pub fn update(&mut self, resp: &Response) -> Option<Gesture> {
        // egui forgets the press origin on the pass that reports the release,
        // so remember where and when the press started ourselves.
        let (now, pos, down) = resp.ctx.input(|i| {
            (
                i.time,
                i.pointer.interact_pos(),
                i.pointer.primary_down() || i.pointer.secondary_down(),
            )
        });
        if down && resp.is_pointer_button_down_on() {
            let Some(p) = self.press.as_mut() else {
                self.press = pos.map(|from| Press {
                    from,
                    t0: now,
                    moved: false,
                    live: None,
                });
                return None;
            };
            let at = pos.unwrap_or(p.from);
            if let Some(last) = p.live {
                if at.distance(last) >= MOVE_MIN_DISTANCE {
                    p.live = Some(at);
                    return Some(Gesture::Move(at));
                }
                return None;
            }
            p.moved |= at.distance(p.from) > SWIPE_MIN_DISTANCE;
            let primary = resp.ctx.input(|i| i.pointer.primary_down());
            if primary && !p.moved {
                let held = ((now - p.t0) * 1000.0) as u32;
                if held >= HOLD_MS {
                    p.live = Some(p.from);
                    return Some(Gesture::Down(p.from));
                }
                // Wake up when the hold is due, even if the pointer is still.
                resp.ctx
                    .request_repaint_after(std::time::Duration::from_millis(
                        u64::from(HOLD_MS - held) + 5,
                    ));
            }
            return None;
        }
        // A quick tap (trackpad tap-to-click) can press and release within one
        // frame, so the press was never seen as "down": use egui's click.
        let Some(Press { from, t0, live, .. }) = self.press.take() else {
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
        if resp.secondary_clicked() {
            return Some(Gesture::LongPress(from));
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

    #[test]
    fn skips_moves_that_a_newer_move_replaces() {
        let (tx, rx) = std::sync::mpsc::channel();
        let cmd = |s: &str, c: &str| (s.to_string(), c.to_string());
        for c in [
            cmd("a", "input motionevent DOWN 1 1"),
            cmd("a", "input motionevent MOVE 2 2"),
            cmd("a", "input motionevent MOVE 3 3"),
            cmd("b", "input motionevent MOVE 9 9"),
            cmd("a", "input motionevent MOVE 4 4"),
            cmd("a", "input motionevent MOVE 5 5"),
            cmd("a", "input motionevent UP 5 5"),
        ] {
            tx.send(c).unwrap();
        }
        let mut queue = std::collections::VecDeque::new();
        let mut run = Vec::new();
        while let Some((s, c)) = next_command(&mut queue, &rx) {
            run.push(format!("{s} {c}"));
        }
        assert_eq!(
            run,
            [
                "a input motionevent DOWN 1 1",
                "a input motionevent MOVE 3 3",
                "b input motionevent MOVE 9 9",
                "a input motionevent MOVE 5 5",
                "a input motionevent UP 5 5",
            ]
        );
    }

    mod gestures {
        use super::super::*;
        use PointerButton::{Primary, Secondary};
        use eframe::egui::{self, Event, PointerButton, RawInput, Rect, Sense, Vec2};

        /// Feed pointer events through real egui passes, one pass per step.
        fn run(steps: &[(f64, Vec<Event>)]) -> Vec<Gesture> {
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
                    out.extend(tracker.update(&resp));
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
            let g = run(&[
                (0.0, vec![mv(100.0, 100.0)]),
                (0.1, vec![button(100.0, 100.0, Primary, true)]),
                (0.2, vec![mv(150.0, 160.0)]),
                (0.3, vec![mv(200.0, 250.0)]),
                (0.5, vec![button(200.0, 250.0, Primary, false)]),
                (0.6, vec![]),
            ]);
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
            let g = run(&[
                (0.0, vec![mv(50.0, 60.0)]),
                (0.1, vec![button(50.0, 60.0, Primary, true)]),
                (0.2, vec![button(50.0, 60.0, Primary, false)]),
                (0.3, vec![]),
            ]);
            assert_eq!(g, vec![Gesture::Tap(Pos2::new(50.0, 60.0))]);
        }

        /// Trackpad tap-to-click delivers press and release in one frame.
        #[test]
        fn press_and_release_in_one_frame_is_a_tap() {
            let g = run(&[
                (0.0, vec![mv(50.0, 60.0)]),
                (
                    0.1,
                    vec![
                        button(50.0, 60.0, Primary, true),
                        button(50.0, 60.0, Primary, false),
                    ],
                ),
                (0.2, vec![]),
            ]);
            assert_eq!(g, vec![Gesture::Tap(Pos2::new(50.0, 60.0))]);
        }

        /// Holding still is a real long press: down while held, up on release.
        #[test]
        fn holding_still_is_a_live_long_press() {
            let g = run(&[
                (0.0, vec![mv(50.0, 60.0)]),
                (0.1, vec![button(50.0, 60.0, Primary, true)]),
                (0.6, vec![]),
                (1.5, vec![button(50.0, 60.0, Primary, false)]),
                (1.6, vec![]),
            ]);
            assert_eq!(
                g,
                vec![
                    Gesture::Down(Pos2::new(50.0, 60.0)),
                    Gesture::Up(Pos2::new(50.0, 60.0)),
                ]
            );
        }

        /// Hold still, then drag: the finger goes down on the device when the
        /// hold is recognised, follows the pointer live, and lifts on release.
        #[test]
        fn hold_then_drag_moves_a_live_finger() {
            let g = run(&[
                (0.0, vec![mv(50.0, 60.0)]),
                (0.1, vec![button(50.0, 60.0, Primary, true)]),
                (0.3, vec![]),
                (0.6, vec![]),
                (0.7, vec![mv(120.0, 60.0)]),
                (0.8, vec![mv(200.0, 90.0)]),
                (0.9, vec![button(200.0, 90.0, Primary, false)]),
                (1.0, vec![]),
            ]);
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
            let g = run(&[
                (0.0, vec![mv(70.0, 80.0)]),
                (0.1, vec![button(70.0, 80.0, Secondary, true)]),
                (0.2, vec![button(70.0, 80.0, Secondary, false)]),
                (0.3, vec![]),
            ]);
            assert_eq!(g, vec![Gesture::LongPress(Pos2::new(70.0, 80.0))]);
        }
    }
}
