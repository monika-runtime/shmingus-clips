//! The complete Tauri command surface. This file is the frozen contract between
//! the backend modules and the frontend: workers implement the service methods
//! behind these commands, they do not change the signatures.

use std::collections::HashMap;
use std::path::PathBuf;

use tauri::{AppHandle, State};

use crate::error::Result;
use crate::model::{
    AudioDevice, AuthStatus, Clip, EnvironmentReport, GameInfo, RecorderStatus, SavedMonitor,
    SelfTestReport, Settings, ShareInfo, ShareServerStatus,
};
use crate::state::AppState;
use crate::{auth, diagnostics, events, sharing, shortcuts};

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<Settings> {
    Ok(state.settings.get())
}

#[tauri::command]
pub async fn save_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: Settings,
) -> Result<Settings> {
    let previous = state.settings.get();
    let next = state.settings.replace(settings)?;

    if previous.launch_at_login != next.launch_at_login {
        apply_autostart(&app, next.launch_at_login);
    }
    if previous.hotkeys != next.hotkeys {
        if let Err(err) = shortcuts::apply(&app).await {
            log::warn!("could not rebind global shortcuts: {err}");
            events::toast(
                &app,
                "info",
                "Hotkeys could not be rebound; use the buttons in the app.",
            );
        }
    }
    if previous.save_dir != next.save_dir {
        events::library_changed(&app);
    }
    if let Err(err) = state.recorder.apply_settings(&app).await {
        log::warn!("could not apply capture settings: {err}");
    }

    Ok(next)
}

/// Base URL of the loopback media server used by `<video>` (WebKitGTK's media
/// stack cannot read the asset protocol).
#[tauri::command]
pub async fn get_media_base(state: State<'_, AppState>) -> Result<String> {
    Ok(state.media_base.clone())
}

#[tauri::command]
pub async fn check_environment() -> Result<EnvironmentReport> {
    Ok(diagnostics::environment_report())
}

#[tauri::command]
pub async fn run_selftest() -> Result<SelfTestReport> {
    crate::capture::selftest().await
}

#[tauri::command]
pub async fn get_audio_devices() -> Result<Vec<AudioDevice>> {
    Ok(crate::capture::audio::list_devices())
}

#[tauri::command]
pub async fn get_recorder_status(state: State<'_, AppState>) -> Result<RecorderStatus> {
    Ok(state.recorder.status())
}

#[tauri::command]
pub async fn start_buffer(app: AppHandle, state: State<'_, AppState>) -> Result<RecorderStatus> {
    state.recorder.start(&app).await
}

#[tauri::command]
pub async fn stop_buffer(app: AppHandle, state: State<'_, AppState>) -> Result<RecorderStatus> {
    state.recorder.stop(&app).await
}

#[tauri::command]
pub async fn toggle_mic(app: AppHandle, state: State<'_, AppState>) -> Result<Settings> {
    state.recorder.toggle_mic(&app).await?;
    Ok(state.settings.get())
}

#[tauri::command]
pub async fn choose_monitor(app: AppHandle, state: State<'_, AppState>) -> Result<SavedMonitor> {
    state.recorder.choose_monitor(&app).await
}

#[tauri::command]
pub async fn save_clip(
    app: AppHandle,
    state: State<'_, AppState>,
    title: Option<String>,
) -> Result<Clip> {
    let game = state.detector.current();
    state
        .recorder
        .save_clip(&app, game, title.filter(|t| !t.trim().is_empty()))
        .await
}

#[tauri::command]
pub async fn get_detected_game(state: State<'_, AppState>) -> Result<Option<GameInfo>> {
    Ok(state.detector.current())
}

#[tauri::command]
pub async fn list_clips(state: State<'_, AppState>) -> Result<Vec<Clip>> {
    state.library.list(&state.settings.get().save_dir).await
}

#[tauri::command]
pub async fn delete_clip(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<()> {
    // A deleted clip must not leave a public link behind. Remove the share
    // first; if the server cannot confirm it, keep the clip so the user can
    // retry instead of silently leaving a live link.
    if sharing::get_share(&id).is_some() {
        sharing::delete_share(&id).await.map_err(|err| {
            crate::error::AppError::msg(format!(
                "Could not remove this clip's share link: {err}. The clip was not deleted, so the link is not left live by accident. Remove the share first or try again."
            ))
        })?;
    }
    state.library.delete(&id).await?;
    events::library_changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn rename_clip(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    title: String,
) -> Result<Clip> {
    let clip = state.library.rename(&id, &title).await?;
    events::library_changed(&app);
    Ok(clip)
}

#[tauri::command]
pub async fn set_clip_favorite(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    favorite: bool,
) -> Result<Clip> {
    let clip = state.library.set_favorite(&id, favorite).await?;
    events::library_changed(&app);
    Ok(clip)
}

#[tauri::command]
pub async fn trim_clip(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    start_ms: u64,
    end_ms: u64,
) -> Result<Clip> {
    let clip = state.library.trim(&id, start_ms, end_ms).await?;
    events::library_changed(&app);
    Ok(clip)
}

#[tauri::command]
pub async fn export_clip(
    _app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    dest_dir: Option<String>,
) -> Result<String> {
    // The frontend owns the success feedback: it has the returned path and can
    // offer to open the folder, which a plain toast cannot.
    state.library.export(&id, dest_dir.as_deref()).await
}

/// Open the folder holding a file the library just exported. Paths outside the
/// clips folder are refused.
#[tauri::command]
pub async fn reveal_export(state: State<'_, AppState>, path: String) -> Result<()> {
    let target = PathBuf::from(&path);
    let save_dir = PathBuf::from(state.settings.get().save_dir);
    if !target.starts_with(&save_dir) {
        return Err(crate::error::AppError::msg(
            "refusing to open a file outside the clips library",
        ));
    }
    if !target.exists() {
        return Err(crate::error::AppError::msg("that file is no longer there"));
    }
    let parent = target.parent().map(PathBuf::from).unwrap_or(save_dir);
    open_path(parent)
}

#[tauri::command]
pub async fn open_clips_folder(state: State<'_, AppState>) -> Result<()> {
    let dir = state.settings.get().save_dir;
    open_path(PathBuf::from(dir))
}

#[tauri::command]
pub async fn reveal_clip(state: State<'_, AppState>, id: String) -> Result<()> {
    let path = state
        .library
        .find_path(&id)
        .ok_or_else(|| crate::error::AppError::msg("clip not found"))?;
    let parent = path
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    open_path(parent)
}

// ---------------------------------------------------------------------------
// Sharing
// ---------------------------------------------------------------------------

/// Upload a clip to the configured share server. The returned share never
/// carries the server's delete token.
#[tauri::command]
pub async fn share_clip(state: State<'_, AppState>, id: String) -> Result<ShareInfo> {
    let clip = state
        .library
        .get(&id)
        .ok_or_else(|| crate::error::AppError::msg("clip not found"))?;
    let settings = state.settings.get();
    sharing::share_clip(&clip, &settings).await
}

#[tauri::command]
pub async fn get_share(id: String) -> Result<Option<ShareInfo>> {
    Ok(sharing::get_share(&id))
}

#[tauri::command]
pub async fn list_shares() -> Result<HashMap<String, ShareInfo>> {
    Ok(sharing::list_shares())
}

#[tauri::command]
pub async fn delete_share(id: String) -> Result<()> {
    sharing::delete_share(&id).await
}

#[tauri::command]
pub async fn test_share_server(state: State<'_, AppState>) -> Result<ShareServerStatus> {
    let settings = state.settings.get();
    sharing::test_share_server(&settings).await
}

// ---------------------------------------------------------------------------
// Authentik
// ---------------------------------------------------------------------------

/// The current Authentik sign-in. Tokens never cross this boundary.
#[tauri::command]
pub async fn auth_status() -> Result<AuthStatus> {
    Ok(auth::status())
}

/// Sign in through the system browser: opens Authentik's authorization page and
/// returns once the loopback redirect has been exchanged for tokens.
#[tauri::command]
pub async fn auth_login() -> Result<AuthStatus> {
    auth::login().await
}

/// Forget the stored Authentik session. Uploads fall back to the static share
/// token afterwards, if one is configured.
#[tauri::command]
pub async fn auth_logout() -> Result<()> {
    auth::logout()
}

fn open_path(path: PathBuf) -> Result<()> {
    std::fs::create_dir_all(&path)?;
    std::process::Command::new("xdg-open")
        .arg(&path)
        .spawn()
        .map_err(|e| {
            crate::error::AppError::msg(format!("could not open {}: {e}", path.display()))
        })?;
    Ok(())
}

fn apply_autostart(app: &AppHandle, enabled: bool) {
    use tauri_plugin_autostart::ManagerExt;
    let manager = app.autolaunch();
    let result = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    if let Err(err) = result {
        log::warn!("could not update autostart setting: {err}");
    }
}
