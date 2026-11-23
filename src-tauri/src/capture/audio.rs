//! Audio device enumeration (PulseAudio protocol on PipeWire).
//! Owned by the capture task.

use std::process::Command;

use crate::model::AudioDevice;

/// Microphones and monitor (loopback) sources, from `pactl list short sources`.
pub fn list_devices() -> Vec<AudioDevice> {
    let default = default_source();
    let Ok(output) = Command::new("pactl")
        .args(["list", "short", "sources"])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        log::warn!("`pactl list short sources` failed: {}", output.status);
        return Vec::new();
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            // id \t name \t module \t sample-spec \t state
            let mut fields = line.split('\t');
            fields.next()?; // id
            let name = fields.next()?.trim();
            if name.is_empty() {
                return None;
            }
            let is_monitor = name.ends_with(".monitor");
            let label = if let Some(sink) = name.strip_suffix(".monitor") {
                format!("Monitor of {}", prettify(sink))
            } else {
                prettify(name)
            };
            Some(AudioDevice {
                id: name.to_string(),
                label,
                is_monitor,
                is_default: default.as_deref() == Some(name),
            })
        })
        .collect()
}

/// The source PulseAudio considers default, if the server answers.
pub fn default_source() -> Option<String> {
    let output = Command::new("pactl")
        .args(["get-default-source"])
        .output()
        .ok()?;
    if output.status.success() {
        let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !name.is_empty() && name != "@DEFAULT_SOURCE@" {
            return Some(name);
        }
    }

    // Older pactl builds have no `get-default-source`; `info` always reports it.
    let output = Command::new("pactl").arg("info").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .find_map(|line| line.strip_prefix("Default Source:"))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

/// The monitor source of the default output sink - this carries desktop audio.
/// Falls back to PulseAudio's special source name when the sink cannot be
/// resolved.
pub fn desktop_source() -> String {
    if let Ok(output) = Command::new("pactl").args(["get-default-sink"]).output() {
        if output.status.success() {
            let sink = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !sink.is_empty() && sink != "@DEFAULT_SINK@" {
                return format!("{sink}.monitor");
            }
        }
    }
    "@DEFAULT_MONITOR@".to_string()
}

/// Turn `alsa_input.usb-Razer_Inc_Razer_Seiren_Mini_UC2046L03404794-00.mono-fallback`
/// into something close to readable. Falls back to the raw name.
fn prettify(name: &str) -> String {
    let mut rest = name;
    for prefix in [
        "alsa_input.",
        "alsa_output.",
        "bluez_input.",
        "bluez_output.",
    ] {
        if let Some(stripped) = rest.strip_prefix(prefix) {
            rest = stripped;
            break;
        }
    }
    rest = rest.strip_suffix(".mono-fallback").unwrap_or(rest);
    rest = rest.strip_suffix(".analog-stereo").unwrap_or(rest);
    rest = rest.strip_suffix(".analog-mono").unwrap_or(rest);
    let cleaned: String = rest
        .replace(['_', '.'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if cleaned.is_empty() {
        name.to_string()
    } else {
        cleaned
    }
}
