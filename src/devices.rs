//! Discovering emulators and devices, and starting / stopping them.

use crate::sdk::Sdk;
use std::collections::BTreeMap;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, PartialEq)]
pub enum State {
    /// Created in the SDK but not running.
    Stopped,
    /// Running and reachable over adb (serial like `emulator-5554`).
    Running { serial: String },
    /// Seen by adb but not ready yet (booting, offline, unauthorized...).
    Other { serial: String, status: String },
}

#[derive(Debug, Clone)]
pub struct Device {
    /// AVD name for emulators, model/serial for physical devices.
    pub name: String,
    pub is_emulator: bool,
    pub state: State,
}

/// One full scan: installed AVDs merged with whatever adb currently sees.
pub fn scan(sdk: &Sdk) -> Vec<Device> {
    let mut by_name: BTreeMap<String, Device> = BTreeMap::new();

    for avd in list_avds(sdk) {
        by_name.insert(
            avd.clone(),
            Device {
                name: avd,
                is_emulator: true,
                state: State::Stopped,
            },
        );
    }

    for (serial, status) in adb_devices(sdk) {
        let ready = status == "device";
        // Emulators started by the SDK use `emulator-NNNN`; ones started by other
        // tools (e.g. Orca) connect over TCP, so ask the device itself.
        let (model, qemu) = if ready {
            props(sdk, &serial)
        } else {
            (None, false)
        };
        let is_emulator = serial.starts_with("emulator-") || qemu;
        let avd = (ready && serial.starts_with("emulator-"))
            .then(|| avd_name(sdk, &serial))
            .flatten();
        let state = if ready {
            State::Running {
                serial: serial.clone(),
            }
        } else {
            State::Other {
                serial: serial.clone(),
                status,
            }
        };
        match avd {
            // Known AVD: replace its "stopped" entry.
            Some(name) => {
                by_name.insert(
                    name.clone(),
                    Device {
                        name,
                        is_emulator,
                        state,
                    },
                );
            }
            // Anything else: keyed by serial so identical models don't collide.
            None => {
                let name = model.unwrap_or_else(|| serial.clone());
                by_name.insert(
                    format!("~{serial}"),
                    Device {
                        name,
                        is_emulator,
                        state,
                    },
                );
            }
        }
    }

    let mut list: Vec<Device> = by_name.into_values().collect();
    // Running first, then alphabetical.
    list.sort_by_key(|d| (matches!(d.state, State::Stopped), d.name.to_lowercase()));
    list
}

fn run(program: &std::path::Path, args: &[&str]) -> Option<String> {
    let out = Command::new(program)
        .args(args)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn list_avds(sdk: &Sdk) -> Vec<String> {
    let Some(emu) = &sdk.emulator else {
        return vec![];
    };
    run(emu, &["-list-avds"])
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        // The emulator sometimes prints INFO/WARNING lines before the list.
        .filter(|l| !l.is_empty() && !l.contains(' ') && !l.starts_with("INFO"))
        .map(String::from)
        .collect()
}

fn adb_devices(sdk: &Sdk) -> Vec<(String, String)> {
    let Some(adb) = &sdk.adb else { return vec![] };
    run(adb, &["devices"])
        .unwrap_or_default()
        .lines()
        .skip(1)
        .filter_map(|l| {
            let mut parts = l.split_whitespace();
            Some((parts.next()?.to_string(), parts.next()?.to_string()))
        })
        .collect()
}

/// Ask a running emulator which AVD it is (`adb -s <serial> emu avd name`).
fn avd_name(sdk: &Sdk, serial: &str) -> Option<String> {
    let out = run(sdk.adb.as_ref()?, &["-s", serial, "emu", "avd", "name"])?;
    out.lines()
        .next()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Model name and whether the device is an emulator (QEMU), in one adb call.
fn props(sdk: &Sdk, serial: &str) -> (Option<String>, bool) {
    let Some(adb) = &sdk.adb else {
        return (None, false);
    };
    let out = run(
        adb,
        &[
            "-s",
            serial,
            "shell",
            "getprop ro.product.model; getprop ro.kernel.qemu",
        ],
    )
    .unwrap_or_default();
    let mut lines = out.lines().map(str::trim);
    let model = lines.next().filter(|s| !s.is_empty()).map(String::from);
    (model, lines.next() == Some("1"))
}

/// Launch an AVD in the background; the emulator outlives this app.
pub fn start(sdk: &Sdk, avd: &str) -> Result<(), String> {
    let emu = sdk.emulator.as_ref().ok_or("emulator binary not found")?;
    Command::new(emu)
        .args(["-avd", avd])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Shut down a running emulator.
pub fn stop(sdk: &Sdk, serial: &str) -> Result<(), String> {
    let adb = sdk.adb.as_ref().ok_or("adb not found")?;
    Command::new(adb)
        .args(["-s", serial, "emu", "kill"])
        .output()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs against the local SDK; prints what the app would show.
    #[test]
    #[ignore = "needs a local Android SDK"]
    fn scan_local() {
        for d in scan(&Sdk::detect()) {
            println!("{d:?}");
        }
    }
}
