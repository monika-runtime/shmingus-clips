//! Audio device enumeration (PulseAudio protocol on PipeWire).
//! Owned by the capture task.

use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gstreamer as gst;
use gstreamer::prelude::*;

use crate::error::{AppError, Result};
use crate::model::{AudioDevice, MicLevel};

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

/// Measure the microphone for a brief window, for the level meter in Settings.
/// `device` is a PulseAudio/PipeWire source name; `None` means the default
/// source. Returns `None` when nothing can be captured (no pulsesrc plugin, no
/// device, or no samples arrived).
pub async fn mic_level(device: Option<String>) -> Result<Option<MicLevel>> {
    tokio::task::spawn_blocking(move || probe_level(device))
        .await
        .map_err(|err| AppError::Capture(format!("mic level task failed: {err}")))?
}

#[derive(Default)]
struct LevelStats {
    sum_squares: f64,
    peak: f64,
    samples: u64,
}

struct LevelProbe {
    pipeline: gst::Pipeline,
    stats: Arc<Mutex<LevelStats>>,
}

impl LevelProbe {
    fn build(device: Option<&str>) -> Result<Option<Self>> {
        if gst::init().is_err() {
            return Ok(None);
        }

        let pipeline = gst::Pipeline::new();
        let src = match gst::ElementFactory::make("pulsesrc").build() {
            Ok(src) => src,
            Err(_) => {
                log::debug!("mic level: pulsesrc is not available");
                return Ok(None);
            }
        };
        if let Some(device) = device {
            src.set_property("device", device);
        }
        let convert = gst::ElementFactory::make("audioconvert")
            .build()
            .map_err(|err| AppError::Capture(format!("audioconvert unavailable: {err}")))?;
        let resample = gst::ElementFactory::make("audioresample")
            .build()
            .map_err(|err| AppError::Capture(format!("audioresample unavailable: {err}")))?;
        let caps = gst::ElementFactory::make("capsfilter")
            .property(
                "caps",
                gst::Caps::builder("audio/x-raw")
                    .field("format", "F32LE")
                    .field("layout", "interleaved")
                    .field("rate", 48_000i32)
                    .field("channels", 2i32)
                    .build(),
            )
            .build()
            .map_err(|err| AppError::Capture(format!("capsfilter unavailable: {err}")))?;
        let sink = gst::ElementFactory::make("fakesink")
            .property("sync", false)
            .build()
            .map_err(|err| AppError::Capture(format!("fakesink unavailable: {err}")))?;

        pipeline
            .add_many([&src, &convert, &resample, &caps, &sink])
            .map_err(|err| AppError::Capture(format!("could not build the mic meter: {err}")))?;
        gst::Element::link_many([&src, &convert, &resample, &caps, &sink])
            .map_err(|err| AppError::Capture(format!("could not link the mic meter: {err}")))?;

        let stats = Arc::new(Mutex::new(LevelStats::default()));
        let sink_pad = sink
            .static_pad("sink")
            .ok_or_else(|| AppError::Capture("fakesink has no pad".into()))?;
        let probe_stats = stats.clone();
        sink_pad.add_probe(gst::PadProbeType::BUFFER, move |_pad, info| {
            if let Some(buffer) = info.buffer() {
                if let Ok(map) = buffer.map_readable() {
                    let mut stats = probe_stats.lock().expect("level stats poisoned");
                    for chunk in map.as_slice().chunks_exact(4) {
                        let sample =
                            f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) as f64;
                        stats.sum_squares += sample * sample;
                        stats.peak = stats.peak.max(sample.abs());
                        stats.samples += 1;
                    }
                }
            }
            gst::PadProbeReturn::Ok
        });

        Ok(Some(Self { pipeline, stats }))
    }

    fn start(&self) -> Result<()> {
        self.pipeline
            .set_state(gst::State::Playing)
            .map_err(|err| AppError::Capture(format!("could not start the mic meter: {err}")))?;
        Ok(())
    }

    fn stop(&self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }

    /// Level since the previous call, then reset the window.
    fn take_level(&self) -> Option<MicLevel> {
        let mut stats = self.stats.lock().expect("level stats poisoned");
        if stats.samples == 0 {
            return None;
        }
        let level = level_from(&stats);
        *stats = LevelStats::default();
        Some(level)
    }
}

fn level_from(stats: &LevelStats) -> MicLevel {
    let rms = (stats.sum_squares / stats.samples.max(1) as f64).sqrt();
    let to_db = |amplitude: f64| {
        if amplitude <= 1e-6 {
            -90.0
        } else {
            (20.0 * amplitude.log10()).clamp(-90.0, 0.0) as f32
        }
    };
    MicLevel {
        rms_db: to_db(rms),
        peak_db: to_db(stats.peak),
    }
}

fn probe_level(device: Option<String>) -> Result<Option<MicLevel>> {
    let Some(probe) = LevelProbe::build(device.as_deref())? else {
        return Ok(None);
    };
    probe.start()?;
    std::thread::sleep(Duration::from_millis(400));
    let level = probe.take_level();
    probe.stop();
    Ok(level)
}

/// Live microphone levels, for the meter in Settings. Call `stop` when the
/// meter is no longer visible.
pub struct MicMonitor {
    stop: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl MicMonitor {
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// Stream levels about ten times a second; the callback runs on the monitor
/// thread.
pub fn start_monitor<F>(device: Option<String>, on_level: F) -> Option<MicMonitor>
where
    F: Fn(MicLevel) + Send + 'static,
{
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = stop.clone();
    let join = std::thread::spawn(move || {
        let Ok(Some(probe)) = LevelProbe::build(device.as_deref()) else {
            return;
        };
        if probe.start().is_err() {
            return;
        }
        while !thread_stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(80));
            if let Some(level) = probe.take_level() {
                on_level(level);
            }
        }
        probe.stop();
    });
    Some(MicMonitor {
        stop,
        join: Some(join),
    })
}

#[cfg(test)]
mod tests {
    /// Manual check: needs a working microphone and GStreamer plugins.
    #[test]
    #[ignore = "requires a microphone"]
    fn streams_microphone_levels() {
        let levels = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = levels.clone();
        let monitor = super::start_monitor(None, move |level| {
            sink.lock().expect("levels").push(level);
        })
        .expect("monitor starts");
        std::thread::sleep(std::time::Duration::from_millis(700));
        monitor.stop();
        let levels = levels.lock().expect("levels");
        println!("{} level samples, last {:?}", levels.len(), levels.last());
        assert!(levels.len() >= 3, "expected several level updates");
    }

    #[test]
    #[ignore = "requires a microphone"]
    fn measures_the_default_microphone() {
        let level = super::probe_level(None).expect("probe runs");
        println!("mic level: {level:?}");
        assert!(level.is_some(), "no samples arrived from the microphone");
    }
}
