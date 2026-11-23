//! Timeline editor rendering.
//!
//! An edit is a list of segments (each one a range of a library clip) plus text
//! overlays. Rendering is a single ffmpeg filter graph: every segment is
//! trimmed, sped up, cropped and levelled independently, then the segments are
//! concatenated; overlays are drawn on the joined video and the mixed track is
//! rebuilt from the isolated desktop and microphone tracks.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::media::{self, MediaProbe};

/// Longest edit accepted, to keep one render bounded.
const MAX_TOTAL_MS: u64 = 2 * 60 * 60 * 1000;
const MAX_SEGMENTS: usize = 100;
const MAX_OVERLAYS: usize = 50;
const MAX_TEXT_CHARS: usize = 200;
const OUT_FPS: u32 = 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditPlan {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub aspect: Aspect,
    pub segments: Vec<Segment>,
    #[serde(default)]
    pub overlays: Vec<TextOverlay>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Aspect {
    #[default]
    Source,
    Vertical,
    Square,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    pub clip_id: String,
    pub start_ms: u64,
    pub end_ms: u64,
    #[serde(default = "one")]
    pub speed: f64,
    #[serde(default = "one_f32")]
    pub desktop_gain: f32,
    #[serde(default = "one_f32")]
    pub mic_gain: f32,
    #[serde(default)]
    pub mute_desktop: bool,
    #[serde(default)]
    pub mute_mic: bool,
    #[serde(default)]
    pub fade_in_ms: u64,
    #[serde(default)]
    pub fade_out_ms: u64,
    #[serde(default)]
    pub rotation: u16,
}

fn one() -> f64 {
    1.0
}

fn one_f32() -> f32 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextOverlay {
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
    #[serde(default = "half")]
    pub x: f32,
    #[serde(default = "near_bottom")]
    pub y: f32,
    #[serde(default = "default_size")]
    pub size: u32,
    #[serde(default = "default_color")]
    pub color: String,
    #[serde(default)]
    pub boxed: bool,
}

fn half() -> f32 {
    0.5
}
fn near_bottom() -> f32 {
    0.82
}
fn default_size() -> u32 {
    48
}
fn default_color() -> String {
    "#ffffff".into()
}

/// A segment's resolved source clip.
#[derive(Debug, Clone)]
pub struct Source {
    pub path: PathBuf,
    pub duration_ms: u64,
    pub width: u32,
    pub height: u32,
    pub audio_streams: usize,
}

/// Output timeline duration for one segment, in milliseconds.
pub fn segment_output_ms(segment: &Segment) -> u64 {
    let speed = segment.speed.clamp(0.25, 4.0);
    ((segment.end_ms.saturating_sub(segment.start_ms)) as f64 / speed).round() as u64
}

pub fn validate(plan: &EditPlan, sources: &[Source]) -> Result<()> {
    if plan.segments.is_empty() {
        return Err(AppError::msg("the edit has no segments"));
    }
    if plan.segments.len() > MAX_SEGMENTS {
        return Err(AppError::msg(format!(
            "an edit can hold at most {MAX_SEGMENTS} segments"
        )));
    }
    if plan.overlays.len() > MAX_OVERLAYS {
        return Err(AppError::msg(format!(
            "an edit can hold at most {MAX_OVERLAYS} text overlays"
        )));
    }
    if sources.len() != plan.segments.len() {
        return Err(AppError::msg("the edit sources do not line up with its segments"));
    }

    let mut total = 0u64;
    for (segment, source) in plan.segments.iter().zip(sources) {
        if segment.end_ms <= segment.start_ms {
            return Err(AppError::msg("a segment ends before it starts"));
        }
        if source.duration_ms > 0 && segment.end_ms > source.duration_ms {
            return Err(AppError::msg("a segment reaches past the end of its clip"));
        }
        if !(0.25..=4.0).contains(&segment.speed) {
            return Err(AppError::msg("speed must be between 0.25x and 4x"));
        }
        if !(0.0..=2.0).contains(&segment.desktop_gain)
            || !(0.0..=2.0).contains(&segment.mic_gain)
        {
            return Err(AppError::msg("audio levels must be between 0% and 200%"));
        }
        if segment.rotation % 90 != 0 {
            return Err(AppError::msg("rotation must be 0, 90, 180 or 270 degrees"));
        }
        total = total.saturating_add(segment_output_ms(segment));
        if total > MAX_TOTAL_MS {
            return Err(AppError::msg("the edit is longer than two hours"));
        }
    }

    for overlay in &plan.overlays {
        let text = overlay.text.trim();
        if text.is_empty() {
            return Err(AppError::msg("a text overlay has no text"));
        }
        if text.chars().count() > MAX_TEXT_CHARS {
            return Err(AppError::msg(format!(
                "text overlays are limited to {MAX_TEXT_CHARS} characters"
            )));
        }
        if overlay.end_ms <= overlay.start_ms {
            return Err(AppError::msg("a text overlay ends before it starts"));
        }
        if !valid_color(&overlay.color) {
            return Err(AppError::msg("text colour must be #rrggbb"));
        }
        if !(1..=400).contains(&overlay.size) {
            return Err(AppError::msg("text size must be between 1 and 400"));
        }
    }
    Ok(())
}

fn valid_color(color: &str) -> bool {
    color.len() == 7
        && color.starts_with('#')
        && color[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// Render `plan` into `out`. Sources are indexed like the plan's segments.
pub fn render(plan: &EditPlan, sources: &[Source], out: &Path) -> Result<MediaProbe> {
    validate(plan, sources)?;
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let first = &sources[0];
    let (width, height) = match plan.aspect {
        Aspect::Source => (even(first.width.max(2)), even(first.height.max(2))),
        Aspect::Vertical => (1080, 1920),
        Aspect::Square => (1080, 1080),
    };

    let mut filters: Vec<String> = Vec::new();
    let n = plan.segments.len();

    for (index, (segment, source)) in plan.segments.iter().zip(sources).enumerate() {
        let start = segment.start_ms as f64 / 1000.0;
        let end = segment.end_ms as f64 / 1000.0;
        let speed = segment.speed.clamp(0.25, 4.0);
        let duration = segment_output_ms(segment) as f64 / 1000.0;

        // --- video -------------------------------------------------------
        let mut video = format!(
            "[{index}:v]trim=start={start:.3}:end={end:.3},setpts=(PTS-STARTPTS)/{speed:.3}"
        );
        video.push_str(match segment.rotation % 360 {
            90 => ",transpose=1",
            180 => ",transpose=1,transpose=1",
            270 => ",transpose=2",
            _ => "",
        });
        video.push_str(match plan.aspect {
            // Crop to the target ratio before scaling; both branches use a
            // height-driven width so portrait and square crops stay centred.
            Aspect::Vertical => ",crop=ih*9/16:ih",
            Aspect::Square => ",crop=ih:ih",
            Aspect::Source => "",
        });
        video.push_str(&format!(
            ",scale={width}:{height}:force_original_aspect_ratio=decrease,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:black,setsar=1,fps={OUT_FPS}[v{index}]"
        ));
        filters.push(video);

        // --- audio -------------------------------------------------------
        let (desktop_track, mic_track) = match source.audio_streams {
            0 => (None, None),
            1 => (Some(0), None),
            2 => (Some(0), Some(1)),
            _ => (Some(1), Some(2)),
        };
        filters.push(audio_chain(
            index,
            desktop_track,
            start,
            end,
            speed,
            duration,
            if segment.mute_desktop {
                0.0
            } else {
                segment.desktop_gain
            },
            segment.fade_in_ms,
            segment.fade_out_ms,
            &format!("d{index}"),
        ));
        filters.push(audio_chain(
            index,
            mic_track,
            start,
            end,
            speed,
            duration,
            if segment.mute_mic { 0.0 } else { segment.mic_gain },
            segment.fade_in_ms,
            segment.fade_out_ms,
            &format!("m{index}"),
        ));
    }

    let video_inputs: String = (0..n).map(|i| format!("[v{i}]")).collect();
    filters.push(format!("{video_inputs}concat=n={n}:v=1:a=0[vcat]"));
    let desktop_inputs: String = (0..n).map(|i| format!("[d{i}]")).collect();
    filters.push(format!("{desktop_inputs}concat=n={n}:v=0:a=1[dcat]"));
    let mic_inputs: String = (0..n).map(|i| format!("[m{i}]")).collect();
    filters.push(format!("{mic_inputs}concat=n={n}:v=0:a=1[mcat]"));

    // Text overlays, last to first so each label feeds the next.
    let mut video_label = "vcat".to_string();
    if let Some(font) = overlay_font() {
        for (index, overlay) in plan.overlays.iter().enumerate() {
            let color = overlay.color.trim_start_matches('#');
            let boxed = if overlay.boxed {
                ":box=1:boxcolor=black@0.55:boxborderw=10"
            } else {
                ""
            };
            let next = format!("vt{index}");
            filters.push(format!(
                "[{video_label}]drawtext=fontfile='{font}':text='{}':fontcolor=0x{color}:fontsize={}:x=(w-text_w)*{:.3}:y=(h-text_h)*{:.3}{boxed}:enable='between(t,{:.3},{:.3})':expansion=none[{next}]",
                escape_text(&overlay.text),
                overlay.size,
                overlay.x.clamp(0.0, 1.0),
                overlay.y.clamp(0.0, 1.0),
                overlay.start_ms as f64 / 1000.0,
                overlay.end_ms as f64 / 1000.0,
            ));
            video_label = next;
        }
    } else if !plan.overlays.is_empty() {
        log::warn!("no font found; rendering the edit without its text overlays");
    }

    // The isolated tracks are both mapped and mixed, so split them: a filter
    // output can be consumed only once.
    filters.push("[dcat]asplit[desktopout][desktopmix]".into());
    filters.push("[mcat]asplit[micout][micmix]".into());
    filters.push(
        "[desktopmix][micmix]amix=inputs=2:duration=longest:normalize=0,alimiter=limit=0.95:level=false[mix]"
            .into(),
    );

    let graph = filters.join(";");
    let mut args: Vec<std::ffi::OsString> = vec!["-y".into()];
    for source in sources {
        args.push("-i".into());
        args.push(source.path.clone().into_os_string());
    }
    let map_label = |label: &str| format!("[{label}]");
    args.extend([
        "-filter_complex".into(),
        graph.into(),
        "-map".into(),
        map_label(&video_label).into(),
        "-map".into(),
        "[mix]".into(),
        "-map".into(),
        "[desktopout]".into(),
        "-map".into(),
        "[micout]".into(),
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "veryfast".into(),
        "-crf".into(),
        "20".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-c:a".into(),
        "aac".into(),
        "-b:a".into(),
        "192k".into(),
        "-ar".into(),
        "48000".into(),
        "-metadata:s:a:0".into(),
        "handler_name=Mixed audio".into(),
        "-metadata:s:a:1".into(),
        "handler_name=Desktop audio".into(),
        "-metadata:s:a:2".into(),
        "handler_name=Microphone".into(),
        "-movflags".into(),
        "+faststart".into(),
        "-avoid_negative_ts".into(),
        "make_zero".into(),
        out.as_os_str().into(),
    ]);

    let refs: Vec<&std::ffi::OsStr> = args.iter().map(|arg| arg.as_os_str()).collect();
    media::run(ffmpeg_bin(), &refs)?;
    media::probe(out)
}

fn audio_chain(
    index: usize,
    track: Option<usize>,
    start: f64,
    end: f64,
    speed: f64,
    duration: f64,
    gain: f32,
    fade_in_ms: u64,
    fade_out_ms: u64,
    label: &str,
) -> String {
    let mut chain = match track {
        Some(track) => format!(
            "[{index}:a:{track}]atrim=start={start:.3}:end={end:.3},asetpts=PTS-STARTPTS"
        ),
        None => format!(
            "anullsrc=r=48000:cl=stereo,atrim=end={duration:.3},asetpts=PTS-STARTPTS"
        ),
    };
    chain.push_str(&atempo_chain(speed));
    chain.push_str(&format!(",volume={:.3}", gain.clamp(0.0, 2.0)));
    let fade_in = (fade_in_ms as f64 / 1000.0).min(duration);
    if fade_in > 0.0 {
        chain.push_str(&format!(",afade=t=in:st=0:d={fade_in:.3}"));
    }
    let fade_out = (fade_out_ms as f64 / 1000.0).min(duration);
    if fade_out > 0.0 {
        let at = (duration - fade_out).max(0.0);
        chain.push_str(&format!(",afade=t=out:st={at:.3}:d={fade_out:.3}"));
    }
    chain.push_str(&format!("[{label}]"));
    chain
}

/// atempo only accepts 0.5..=2 per instance; chain them for wider speeds.
fn atempo_chain(speed: f64) -> String {
    let mut remaining = speed.clamp(0.25, 4.0);
    let mut links = Vec::new();
    while remaining > 2.0 {
        links.push("atempo=2.0".to_string());
        remaining /= 2.0;
    }
    while remaining < 0.5 {
        links.push("atempo=0.5".to_string());
        remaining /= 0.5;
    }
    links.push(format!("atempo={remaining:.3}"));
    format!(",{}", links.join(","))
}

/// The distro ffmpeg is the one built with drawtext/libass; a Homebrew or
/// custom build earlier in PATH may lack them, so prefer the system binary
/// when it exists.
fn ffmpeg_bin() -> &'static str {
    if Path::new("/usr/bin/ffmpeg").is_file() {
        "/usr/bin/ffmpeg"
    } else {
        "ffmpeg"
    }
}

fn even(value: u32) -> u32 {
    value & !1
}

fn escape_text(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace(':', "\\:")
        .replace('\n', " ")
}

fn overlay_font() -> Option<String> {
    if let Ok(output) = std::process::Command::new("fc-match")
        .args(["-f", "%{file}", "sans"])
        .output()
    {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() && Path::new(&path).is_file() {
                return Some(path.replace('\\', "\\\\").replace(':', "\\:").replace('\'', "\\'"));
            }
        }
    }
    [
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
    ]
    .into_iter()
    .find(|path| Path::new(path).is_file())
    .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atempo_covers_the_whole_supported_range() {
        assert_eq!(atempo_chain(1.0), ",atempo=1.000");
        assert_eq!(atempo_chain(0.25), ",atempo=0.5,atempo=0.500");
        assert_eq!(atempo_chain(4.0), ",atempo=2.0,atempo=2.000");
    }

    #[test]
    fn rejects_an_empty_edit_before_touching_ffmpeg() {
        let plan = EditPlan {
            title: String::new(),
            aspect: Aspect::Source,
            segments: Vec::new(),
            overlays: Vec::new(),
        };
        assert!(validate(&plan, &[]).is_err());
    }

    #[test]
    fn escapes_drawtext_metacharacters() {
        assert_eq!(escape_text("50% 'yes': no"), "50% \\'yes\\'\\: no");
    }

    #[test]
    fn segment_output_accounts_for_speed() {
        let segment = Segment {
            clip_id: "x".into(),
            start_ms: 0,
            end_ms: 4000,
            speed: 2.0,
            desktop_gain: 1.0,
            mic_gain: 1.0,
            mute_desktop: false,
            mute_mic: false,
            fade_in_ms: 0,
            fade_out_ms: 0,
            rotation: 0,
        };
        assert_eq!(segment_output_ms(&segment), 2000);
    }
}
