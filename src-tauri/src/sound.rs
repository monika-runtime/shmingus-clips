//! Audible confirmation when a clip is saved.
//!
//! Desktop notifications are often suppressed while a fullscreen game has
//! focus, so the app plays a short chime itself. GStreamer is already a
//! dependency; the WAV is embedded in the binary and written to the cache on
//! first use, then played on a dedicated thread so nothing blocks the save.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gstreamer::prelude::*;

use crate::error::{AppError, Result};

const CHIME: &[u8] = include_bytes!("../resources/clip-saved.wav");

/// Play the clip-saved chime. Never blocks the caller and never fails loudly.
pub fn play_clip_saved() {
    let spawned = std::thread::Builder::new()
        .name("shmingus-clips-sound".into())
        .spawn(|| {
            if let Err(err) = play() {
                log::warn!("could not play the clip sound: {err}");
            }
        });
    if let Err(err) = spawned {
        log::warn!("could not start the sound thread: {err}");
    }
}

fn play() -> Result<()> {
    let path = ensure_chime()?;

    gstreamer::init()
        .map_err(|err| AppError::msg(format!("could not initialise GStreamer: {err}")))?;

    let uri = gstreamer::glib::filename_to_uri(&path, None)
        .map_err(|err| AppError::msg(format!("invalid sound path: {err}")))?;

    let playbin = gstreamer::ElementFactory::make("playbin")
        .property("uri", uri.as_str())
        .build()
        .map_err(|err| AppError::msg(format!("could not build the sound player: {err}")))?;
    playbin.set_property("volume", 0.6f64);
    playbin
        .set_state(gstreamer::State::Playing)
        .map_err(|err| AppError::msg(format!("could not start the sound: {err}")))?;

    let bus = playbin
        .bus()
        .ok_or_else(|| AppError::msg("the sound player has no bus"))?;
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut outcome = Ok(());
    while Instant::now() < deadline {
        let Some(message) = bus.timed_pop(gstreamer::ClockTime::from_mseconds(100)) else {
            continue;
        };
        match message.view() {
            gstreamer::MessageView::Eos(_) => break,
            gstreamer::MessageView::Error(err) => {
                outcome = Err(AppError::msg(format!(
                    "sound playback failed: {} ({})",
                    err.error(),
                    err.debug().unwrap_or_else(|| "no details".into())
                )));
                break;
            }
            _ => {}
        }
    }

    let _ = playbin.set_state(gstreamer::State::Null);
    outcome
}

/// Write the embedded chime to the cache once (or when the asset changes).
fn ensure_chime() -> Result<PathBuf> {
    let dir = crate::settings::cache_dir().join("sounds");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("clip-saved.wav");

    let current = std::fs::metadata(&path).map(|meta| meta.len()).ok();
    if current != Some(CHIME.len() as u64) {
        std::fs::write(&path, CHIME)?;
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::CHIME;

    #[test]
    fn the_embedded_chime_is_a_wav() {
        assert!(CHIME.len() > 1_000, "chime looks empty");
        assert_eq!(&CHIME[0..4], b"RIFF");
        assert_eq!(&CHIME[8..12], b"WAVE");
    }
}
