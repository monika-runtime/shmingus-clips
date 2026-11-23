//! Turning buffered segments into a finished MP4 clip: split-now, concat the
//! last N seconds, remux to MP4 keeping separate audio tracks, thumbnail,
//! metadata and notification.
//! Owned by the capture task.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tauri::AppHandle;
use tauri::Manager;
use uuid::Uuid;

use crate::error::{AppError, Result};
use crate::events;
use crate::media::{self, MediaProbe};
use crate::model::{Clip, GameInfo, Settings};
use crate::state::AppState;

use super::pipeline::RunningPipeline;
use super::ring::SegmentRing;

/// Build the clip file for the buffered segments: split the live fragment
/// open, wait until it is closed, concat everything, remux (keeping all audio
/// tracks) and generate a thumbnail.
pub async fn save(
    app: &AppHandle,
    pipeline: &RunningPipeline,
    ring: &SegmentRing,
    settings: &Settings,
    game: Option<GameInfo>,
    title: Option<String>,
) -> Result<Clip> {
    let last_index = ring
        .latest_index()
        .ok_or_else(|| AppError::Capture("the replay buffer has no footage yet".into()))?;

    pipeline.split_now()?;
    let split = wait_for_new_segment(ring, last_index, Duration::from_millis(2500)).await;
    if !split {
        log::warn!("splitmuxsink did not open a new fragment in time; saving what is closed");
    }
    // Give the sink a moment to finish writing the fragment it just closed.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let parts = ring.tail_up_to(settings.buffer_seconds, last_index);
    if parts.is_empty() {
        return Err(AppError::Capture("the replay buffer is empty".into()));
    }

    let game_name = game.as_ref().map(|game| game.name.clone());
    // A timestamp-only title is useless once there are a few clips: name it
    // after the game (or the desktop) and the moment, so two clips saved
    // minutes apart are distinguishable in the library.
    let title = title.unwrap_or_else(|| {
        let source = game_name.as_deref().unwrap_or("Desktop");
        format!("{source} clip")
    });

    let dest_dir = PathBuf::from(&settings.save_dir).join(sanitize_component(
        game_name.as_deref().unwrap_or("General"),
    ));
    std::fs::create_dir_all(&dest_dir)?;

    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let slug = slugify(&title);
    let mut out_path = dest_dir.join(format!("{slug}-{stamp}.mp4"));
    if out_path.exists() {
        out_path = dest_dir.join(format!(
            "{slug}-{stamp}-{}.mp4",
            &Uuid::new_v4().to_string()[..8]
        ));
    }

    let thumbnail = crate::settings::cache_dir()
        .join("thumbnails")
        .join(format!("{}.jpg", Uuid::new_v4()));
    let work_dir = crate::settings::cache_dir()
        .join("clips")
        .join(Uuid::new_v4().to_string());

    let requested_ms = settings.buffer_seconds as u64 * 1000;
    let probe = {
        let parts = parts.clone();
        let work_dir = work_dir.clone();
        let out_path = out_path.clone();
        let thumbnail = thumbnail.clone();
        tokio::task::spawn_blocking(move || {
            assemble(&parts, &work_dir, &out_path, &thumbnail, requested_ms)
        })
        .await
        .map_err(|err| AppError::Capture(format!("clip render task failed: {err}")))??
    };

    let clip = Clip {
        id: Uuid::new_v4().to_string(),
        title,
        game: game_name,
        path: out_path.to_string_lossy().to_string(),
        thumbnail: Some(thumbnail.to_string_lossy().to_string()),
        created_at: chrono::Utc::now().to_rfc3339(),
        duration_ms: probe.duration_ms,
        size_bytes: probe.size_bytes,
        width: probe.width,
        height: probe.height,
        favorite: false,
    };

    let state = app.state::<AppState>();
    state.library.upsert(clip.clone()).await?;
    events::clip_saved(app, &clip);

    if settings.notify_on_clip {
        notify(&clip);
    }

    Ok(clip)
}

/// Concat `parts` into a single TS, remux it to `out_mp4` and write a JPEG
/// thumbnail. Used by both the live saver and the headless selftest.
pub fn assemble(
    parts: &[PathBuf],
    work_dir: &Path,
    out_mp4: &Path,
    thumbnail: &Path,
    requested_ms: u64,
) -> Result<MediaProbe> {
    if parts.is_empty() {
        return Err(AppError::Capture("there are no segments to save".into()));
    }
    std::fs::create_dir_all(work_dir)?;
    if let Some(parent) = out_mp4.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let result = (|| {
        let concat_path = work_dir.join("concat.ts");
        concat_all(parts, &concat_path)?;

        let joined = media::probe(&concat_path)?;
        if joined.duration_ms == 0 {
            return Err(AppError::Media(
                "the concatenated buffer has no duration".into(),
            ));
        }
        let duration_ms = if requested_ms == 0 {
            joined.duration_ms
        } else {
            requested_ms.min(joined.duration_ms)
        };
        media::remux_clip_mp4(
            &concat_path,
            out_mp4,
            0,
            duration_ms,
            joined.audio_streams,
        )?;
        let _ = std::fs::remove_file(&concat_path);

        let probe = media::probe(out_mp4)?;
        let frame_at = 500.min(probe.duration_ms / 2);
        media::thumbnail(out_mp4, thumbnail, frame_at, 640)?;
        Ok(probe)
    })();

    let _ = std::fs::remove_dir_all(work_dir);
    if result.is_err() {
        // Never leave a half-written clip behind.
        let _ = std::fs::remove_file(out_mp4);
    }
    result
}

/// `media::concat_ts` cannot be used here: ffmpeg's default stream selection
/// keeps only one audio stream, which would silently drop the microphone
/// track. `-map 0` keeps every stream.
fn concat_all(parts: &[PathBuf], out: &Path) -> Result<()> {
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let list_path = out.with_extension("concat.txt");
    let mut list = String::new();
    for part in parts {
        let display = part.to_string_lossy().replace('\'', "'\\''");
        list.push_str(&format!("file '{display}'\n"));
    }
    std::fs::write(&list_path, list)?;

    let result = media::run(
        "ffmpeg",
        &[
            "-y".as_ref(),
            "-f".as_ref(),
            "concat".as_ref(),
            "-safe".as_ref(),
            "0".as_ref(),
            "-i".as_ref(),
            list_path.as_os_str(),
            "-map".as_ref(),
            "0".as_ref(),
            "-c".as_ref(),
            "copy".as_ref(),
            out.as_os_str(),
        ],
    );
    let _ = std::fs::remove_file(&list_path);
    result?;
    Ok(())
}

/// Poll until splitmuxsink has opened a fragment after `previous`. Returns
/// false on timeout (the caller then saves the fragments that are closed).
pub async fn wait_for_new_segment(ring: &SegmentRing, previous: u32, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if ring
            .latest_index()
            .map(|index| index > previous)
            .unwrap_or(false)
        {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Desktop notification.
///
/// `notify-rust` blocks on a zbus connection which, under the zbus `tokio`
/// feature, needs the Tokio runtime. Calling `show()` on a Tokio worker (the
/// save command runs there) deadlocks that worker - the connection task can
/// never make progress. Give the notification its own OS thread instead.
fn notify(clip: &Clip) {
    let title = clip.title.clone();
    let spawned = std::thread::Builder::new()
        .name("clipforge-notify".into())
        .spawn(move || {
            let result = notify_rust::Notification::new()
                .summary("Clip saved")
                .body(&title)
                .icon("media-record")
                .timeout(notify_rust::Timeout::Milliseconds(4000))
                .show();
            if let Err(err) = result {
                log::warn!("could not show the clip notification: {err}");
            }
        });
    if let Err(err) = spawned {
        log::warn!("could not start the notification thread: {err}");
    }
}

fn sanitize_component(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '\0' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').trim();
    if cleaned.is_empty() {
        "General".to_string()
    } else {
        cleaned.to_string()
    }
}

fn slugify(value: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = false;
    for c in value.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
        if slug.len() >= 60 {
            break;
        }
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "clip".to_string()
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::{sanitize_component, slugify};

    #[test]
    fn slugs_are_filesystem_safe() {
        assert_eq!(slugify("Elden Ring - Boss Fight!"), "elden-ring-boss-fight");
        assert_eq!(slugify("///"), "clip");
        assert_eq!(sanitize_component("A/B:C"), "A_B_C");
        assert_eq!(sanitize_component("   "), "General");
    }
}
