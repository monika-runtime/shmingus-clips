use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::model::{Clip, GameInfo, RecorderStatus, Toast};

pub const RECORDER_STATUS: &str = "recorder:status";
pub const CLIP_SAVED: &str = "clip:saved";
pub const GAME_DETECTED: &str = "game:detected";
pub const LIBRARY_CHANGED: &str = "library:changed";
pub const TOAST: &str = "toast";

pub fn emit<T: Serialize + Clone>(app: &AppHandle, event: &str, payload: T) {
    if let Err(err) = app.emit(event, payload) {
        log::warn!("failed to emit {event}: {err}");
    }
}

pub fn recorder_status(app: &AppHandle, status: &RecorderStatus) {
    emit(app, RECORDER_STATUS, status.clone());
}

pub fn clip_saved(app: &AppHandle, clip: &Clip) {
    emit(app, CLIP_SAVED, clip.clone());
    library_changed(app);
}

pub fn game_detected(app: &AppHandle, game: Option<GameInfo>) {
    emit(app, GAME_DETECTED, game);
}

pub fn library_changed(app: &AppHandle) {
    emit(app, LIBRARY_CHANGED, ());
}

pub fn toast(app: &AppHandle, level: &str, message: impl Into<String>) {
    emit(
        app,
        TOAST,
        Toast {
            level: level.to_string(),
            message: message.into(),
        },
    );
}
