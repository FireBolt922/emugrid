//! Locating the Android SDK tools (`adb`, `emulator`) on any OS.

use std::env;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Sdk {
    pub adb: Option<PathBuf>,
    pub emulator: Option<PathBuf>,
}

impl Sdk {
    pub fn detect() -> Self {
        let roots = sdk_roots();
        Self {
            adb: find_tool(&roots, "platform-tools", "adb"),
            emulator: find_tool(&roots, "emulator", "emulator"),
        }
    }
}

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// Candidate SDK roots: env vars first, then the default install locations.
fn sdk_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for var in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Ok(v) = env::var(var) {
            roots.push(PathBuf::from(v));
        }
    }
    if let Some(home) = env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")) {
        let home = PathBuf::from(home);
        roots.push(home.join("Library/Android/sdk")); // macOS
        roots.push(home.join("Android/Sdk")); // Linux
    }
    if let Some(local) = env::var_os("LOCALAPPDATA") {
        roots.push(PathBuf::from(local).join("Android/Sdk")); // Windows
    }
    roots
}

fn find_tool(roots: &[PathBuf], subdir: &str, name: &str) -> Option<PathBuf> {
    let file = exe(name);
    roots
        .iter()
        .map(|r| r.join(subdir).join(&file))
        .find(|p| p.is_file())
        .or_else(|| on_path(&file))
}

fn on_path(file: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|d| d.join(file))
        .find(|p| Path::new(p).is_file())
}
