//! Thin, synchronous wrappers around the `ffmpeg` / `ffprobe` binaries.
//!
//! These block, so async callers should wrap them in
//! `tokio::task::spawn_blocking` when the file is large.

use std::path::Path;
use std::process::Command;

use crate::error::{AppError, Result};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaProbe {
    pub duration_ms: u64,
    pub width: u32,
    pub height: u32,
    pub audio_streams: usize,
    pub size_bytes: u64,
}

/// Only used by tests to skip fixtures when ffmpeg is absent.
#[cfg(test)]
pub fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn ffprobe_available() -> bool {
    Command::new("ffprobe")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn run(program: &str, args: &[&std::ffi::OsStr]) -> Result<String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| AppError::Media(format!("could not run {program}: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: String = stderr
            .lines()
            .rev()
            .take(8)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join(" | ");
        return Err(AppError::Media(format!(
            "{program} exited with {}: {tail}",
            output.status
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Probe duration, dimensions, audio track count and size of a media file.
pub fn probe(path: &Path) -> Result<MediaProbe> {
    let raw = run(
        "ffprobe",
        &[
            "-v".as_ref(),
            "error".as_ref(),
            "-print_format".as_ref(),
            "json".as_ref(),
            "-show_format".as_ref(),
            "-show_streams".as_ref(),
            path.as_os_str(),
        ],
    )?;

    let json: serde_json::Value = serde_json::from_str(&raw)?;
    let duration = json
        .pointer("/format/duration")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    let size_bytes = json
        .pointer("/format/size")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<u64>().ok())
        .or_else(|| std::fs::metadata(path).ok().map(|m| m.len()))
        .unwrap_or(0);

    let mut width = 0;
    let mut height = 0;
    let mut audio_streams = 0;
    if let Some(streams) = json.get("streams").and_then(|v| v.as_array()) {
        for stream in streams {
            match stream.get("codec_type").and_then(|v| v.as_str()) {
                Some("video") if width == 0 => {
                    width = stream.get("width").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                    height = stream.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                }
                Some("audio") => audio_streams += 1,
                _ => {}
            }
        }
    }

    Ok(MediaProbe {
        duration_ms: (duration * 1000.0).round() as u64,
        width,
        height,
        audio_streams,
        size_bytes,
    })
}

/// Extract a single JPEG frame at `at_ms`, scaled to `width` px (keeps aspect).
pub fn thumbnail(video: &Path, out: &Path, at_ms: u64, width: u32) -> Result<()> {
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let seconds = format!("{:.3}", at_ms as f64 / 1000.0);
    let scale = format!("scale={width}:-2");
    run(
        "ffmpeg",
        &[
            "-y".as_ref(),
            "-ss".as_ref(),
            seconds.as_ref(),
            "-i".as_ref(),
            video.as_os_str(),
            "-frames:v".as_ref(),
            "1".as_ref(),
            "-vf".as_ref(),
            scale.as_ref(),
            "-q:v".as_ref(),
            "3".as_ref(),
            out.as_os_str(),
        ],
    )?;
    Ok(())
}

/// Remux a TS/MP4 file into an MP4, copying every stream (all audio tracks).
/// `duration_ms == 0` means "to the end".
pub fn remux_mp4(input: &Path, out: &Path, start_ms: u64, duration_ms: u64) -> Result<()> {
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let start = format!("{:.3}", start_ms as f64 / 1000.0);
    let duration = format!("{:.3}", duration_ms as f64 / 1000.0);

    let mut args: Vec<&std::ffi::OsStr> = vec!["-y".as_ref(), "-ss".as_ref(), start.as_ref()];
    if duration_ms > 0 {
        args.push("-t".as_ref());
        args.push(duration.as_ref());
    }
    args.extend_from_slice(&[
        "-i".as_ref(),
        input.as_os_str(),
        "-map".as_ref(),
        "0".as_ref(),
        "-c".as_ref(),
        "copy".as_ref(),
        "-movflags".as_ref(),
        "+faststart".as_ref(),
        "-avoid_negative_ts".as_ref(),
        "make_zero".as_ref(),
        out.as_os_str(),
    ]);

    run("ffmpeg", &args)?;
    Ok(())
}

/// Remux a freshly assembled clip and put a desktop + microphone mix first, so
/// ordinary players - which only play the first audio track - hear both. The
/// isolated tracks follow it for editing. A clip with any other number of
/// audio tracks is copied unchanged.
pub fn remux_clip_mp4(
    input: &Path,
    out: &Path,
    start_ms: u64,
    duration_ms: u64,
    audio_streams: usize,
    auto_gain: bool,
) -> Result<()> {
    if audio_streams != 2 {
        return remux_mp4(input, out, start_ms, duration_ms);
    }
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let start = format!("{:.3}", start_ms as f64 / 1000.0);
    let duration = format!("{:.3}", duration_ms as f64 / 1000.0);

    // Experimental: a quiet microphone disappears under game audio once the
    // tracks are summed. Measure both tracks and lift the mic toward the
    // desktop level, capped, and only when the mic actually carries signal.
    let gain_db = if auto_gain { auto_gain_db(input) } else { 0.0 };
    let filter = if gain_db > 0.05 {
        format!(
            "[0:a:1]volume={gain_db:.1}dB[mic];\
             [0:a:0][mic]amix=inputs=2:duration=longest:normalize=0,\
             alimiter=limit=0.95:level=false[mix]"
        )
    } else {
        "[0:a:0][0:a:1]amix=inputs=2:duration=longest:normalize=0,\
         alimiter=limit=0.95:level=false[mix]"
            .to_string()
    };

    let mut args: Vec<&std::ffi::OsStr> = vec!["-y".as_ref(), "-ss".as_ref(), start.as_ref()];
    if duration_ms > 0 {
        args.push("-t".as_ref());
        args.push(duration.as_ref());
    }
    args.extend_from_slice(&[
        "-i".as_ref(),
        input.as_os_str(),
        // normalize=0 keeps the recorded levels; the limiter only stops the sum
        // from clipping when game audio and voice peak together.
        "-filter_complex".as_ref(),
        filter.as_ref(),
        "-map".as_ref(),
        "0:v:0".as_ref(),
        "-map".as_ref(),
        "[mix]".as_ref(),
        "-map".as_ref(),
        "0:a:0".as_ref(),
        "-map".as_ref(),
        "0:a:1".as_ref(),
        "-c:v".as_ref(),
        "copy".as_ref(),
        "-c:a:0".as_ref(),
        "aac".as_ref(),
        "-b:a:0".as_ref(),
        "192k".as_ref(),
        "-c:a:1".as_ref(),
        "copy".as_ref(),
        "-c:a:2".as_ref(),
        "copy".as_ref(),
        "-metadata:s:a:0".as_ref(),
        "handler_name=Mixed audio".as_ref(),
        "-metadata:s:a:1".as_ref(),
        "handler_name=Desktop audio".as_ref(),
        "-metadata:s:a:2".as_ref(),
        "handler_name=Microphone".as_ref(),
        "-movflags".as_ref(),
        "+faststart".as_ref(),
        "-avoid_negative_ts".as_ref(),
        "make_zero".as_ref(),
        out.as_os_str(),
    ]);

    run("ffmpeg", &args)?;
    Ok(())
}

/// How much to lift the microphone in the mix, in dB. Zero when the setting is
/// off, when either track cannot be measured, or when the mic is quiet enough
/// to be silence (boosting noise would be worse than leaving it).
fn auto_gain_db(input: &Path) -> f32 {
    const SILENT_DB: f32 = -55.0;
    const MAX_GAIN_DB: f32 = 12.0;

    let (Some(desktop), Some(mic)) = (mean_volume_db(input, 0), mean_volume_db(input, 1)) else {
        return 0.0;
    };
    if mic < SILENT_DB {
        return 0.0;
    }
    let gain = (desktop - mic).clamp(0.0, MAX_GAIN_DB);
    if gain >= 0.5 {
        log::info!(
            "microphone auto gain: desktop {desktop:.1} dB, mic {mic:.1} dB, boosting {gain:.1} dB"
        );
    }
    gain
}

fn mean_volume_db(input: &Path, track: usize) -> Option<f32> {
    let map = format!("0:a:{track}");
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-nostats", "-i"])
        .arg(input)
        .args(["-map", &map, "-af", "volumedetect", "-f", "null", "-"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stderr);
    text.lines()
        .find_map(|line| line.split("mean_volume:").nth(1))
        .and_then(|value| value.trim().strip_suffix(" dB"))
        .and_then(|value| value.trim().parse::<f32>().ok())
}

/// Copy a finished clip somewhere else, keeping every stream intact.
pub fn copy_clip(input: &Path, out: &Path) -> Result<()> {
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(input, out)?;
    Ok(())
}

pub fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}
