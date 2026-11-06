//! Locating the Android SDK tools (`adb`, `emulator`) on any OS.

use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone)]
pub struct Sdk {
    pub adb: Option<PathBuf>,
    pub emulator: Option<PathBuf>,
    /// `xcrun simctl` works (macOS with Xcode installed).
    pub simctl: bool,
}

impl Sdk {
    pub fn detect() -> Self {
        let roots = sdk_roots();
        Self {
            adb: find_tool(&roots, "platform-tools", "adb"),
            emulator: find_tool(&roots, "emulator", "emulator"),
            simctl: cfg!(target_os = "macos")
                && command("xcrun")
                    .args(["--find", "simctl"])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .is_ok_and(|s| s.success()),
        }
    }
}

/// A `Command` that never flashes a console window on Windows.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.stdin(Stdio::null());
    cmd
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
