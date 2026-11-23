//! Locating the Android SDK tools (`adb`, `emulator`) on any OS.

use std::env;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone)]
pub struct Sdk {
    pub root: Option<PathBuf>,
    pub adb: Option<PathBuf>,
    pub emulator: Option<PathBuf>,
    /// `xcrun simctl` works (macOS with Xcode installed).
    pub simctl: bool,
}

pub struct Check {
    pub label: &'static str,
    pub path: Option<PathBuf>,
    pub result: &'static str,
    pub found: bool,
}

impl Sdk {
    pub fn detect(override_root: Option<&Path>) -> Self {
        let env = |key: &str| env::var_os(key);
        let roots = sdk_roots(override_root, &env);
        let adb = find_tool(&roots, "platform-tools", "adb", &env);
        let emulator = find_tool(&roots, "emulator", "emulator", &env);
        let root = adb
            .as_deref()
            .and_then(|p| tool_root(p, "platform-tools"))
            .or_else(|| emulator.as_deref().and_then(|p| tool_root(p, "emulator")));
        Self {
            root,
            adb,
            emulator,
            // `xcrun` is a stub that pops an "install developer tools" dialog
            // when none are installed, so ask `xcode-select` (silent) first.
            simctl: cfg!(target_os = "macos")
                && command("xcode-select")
                    .arg("-p")
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .is_ok_and(|s| s.success())
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

type Candidate = (&'static str, Option<PathBuf>);

fn sdk_roots(
    override_root: Option<&Path>,
    env: &impl Fn(&str) -> Option<OsString>,
) -> Vec<Candidate> {
    let get = |key| {
        env(key)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    let home = get("HOME").or_else(|| get("USERPROFILE"));
    let mut roots = vec![
        ("$ANDROID_HOME", get("ANDROID_HOME")),
        ("$ANDROID_SDK_ROOT", get("ANDROID_SDK_ROOT")),
        (
            "~/Library/Android/sdk",
            home.as_ref().map(|p| p.join("Library/Android/sdk")),
        ),
        ("~/Android/Sdk", home.map(|p| p.join("Android/Sdk"))),
    ];
    if cfg!(windows) {
        roots.push((
            "%LOCALAPPDATA%/Android/Sdk",
            get("LOCALAPPDATA").map(|p| p.join("Android/Sdk")),
        ));
    }
    if let Some(path) = override_root {
        roots.insert(0, ("Selected SDK folder", Some(path.into())));
    }
    roots
}

fn find_tool(
    roots: &[Candidate],
    subdir: &str,
    name: &str,
    env: &impl Fn(&str) -> Option<OsString>,
) -> Option<PathBuf> {
    let file = exe(name);
    roots
        .iter()
        .filter_map(|(_, path)| Some(path.as_ref()?.join(subdir).join(&file)))
        .find(|p| usable_tool(p))
        .or_else(|| on_path(&file, env))
}

fn on_path(file: &str, env: &impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    env::split_paths(&env("PATH")?)
        .map(|path| path.join(file))
        .find(|p| usable_tool(p))
}

fn usable_tool(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return false;
        }
    }
    metadata.is_file()
}

fn tool_root(path: &Path, subdir: &str) -> Option<PathBuf> {
    let dir = path.parent()?;
    (dir.file_name()? == subdir)
        .then(|| dir.parent().map(Path::to_path_buf))
        .flatten()
}

pub fn resolve_path(path: &Path) -> std::io::Result<PathBuf> {
    if let Ok(rest) = path.strip_prefix("~")
        && let Some(home) = env::var_os("HOME").or_else(|| env::var_os("USERPROFILE"))
    {
        return std::path::absolute(PathBuf::from(home).join(rest));
    }
    std::path::absolute(path)
}

pub fn has_adb(root: &Path) -> bool {
    usable_tool(&root.join("platform-tools").join(exe("adb")))
}

pub fn diagnose(override_root: Option<&Path>) -> Vec<Check> {
    diagnose_with(override_root, &|key| env::var_os(key))
}

fn diagnose_with(
    override_root: Option<&Path>,
    env: &impl Fn(&str) -> Option<OsString>,
) -> Vec<Check> {
    let mut checks: Vec<_> = sdk_roots(override_root, env)
        .into_iter()
        .map(|(label, path)| {
            let (result, found) = match path.as_deref() {
                None => ("Not set", false),
                Some(p) if !p.is_dir() => ("No such folder", false),
                Some(p) if has_adb(p) => ("adb found", true),
                Some(_) => ("platform-tools/adb missing or not executable", false),
            };
            Check {
                label,
                path,
                result,
                found,
            }
        })
        .collect();
    for (label, name) in [("adb on $PATH", "adb"), ("emulator on $PATH", "emulator")] {
        let path = on_path(&exe(name), env);
        let found = path.is_some();
        checks.push(Check {
            label,
            path,
            found,
            result: if found { "Found" } else { "Not found" },
        });
    }
    checks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnoses_missing_roots_and_prefers_the_override() {
        let dir = std::env::temp_dir().join(format!("emugrid-sdk-{}", std::process::id()));
        let tool = dir.join("platform-tools").join(exe("adb"));
        std::fs::create_dir_all(tool.parent().unwrap()).unwrap();
        std::fs::write(&tool, []).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let env = |key: &str| (key == "HOME").then(|| dir.join("no-home").into_os_string());
        let checks = diagnose_with(Some(&dir), &env);
        assert!(checks[0].found);
        assert_eq!(checks[0].label, "Selected SDK folder");
        assert_eq!(checks[1].result, "Not set");
        assert_eq!(checks[4].result, "No such folder");
        assert_eq!(
            find_tool(&sdk_roots(Some(&dir), &env), "platform-tools", "adb", &env),
            Some(tool)
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
