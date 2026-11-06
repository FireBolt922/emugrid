//! Screen capture backends. Each running device gets one worker thread that
//! pushes `Frame`s; the UI only ever sees the latest frame per device.

pub mod grpc;
pub mod png;
pub mod proto;

use std::sync::atomic::{AtomicBool, AtomicU32};

/// One screen image, tightly packed RGB.
#[derive(Debug, Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

/// Shared knobs between the UI and a capture worker.
#[derive(Debug)]
pub struct Control {
    /// Worker should exit.
    pub stop: AtomicBool,
    /// Tile is on screen; when false the worker idles.
    pub active: AtomicBool,
    /// Longest edge, in pixels, the UI wants frames at.
    pub size: AtomicU32,
    pub max_fps: AtomicU32,
}

impl Control {
    pub fn new(size: u32) -> Self {
        Self {
            stop: AtomicBool::new(false),
            active: AtomicBool::new(true),
            size: AtomicU32::new(size),
            max_fps: AtomicU32::new(15),
        }
    }
}
