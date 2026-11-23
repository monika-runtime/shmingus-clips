mod auth;
mod capture;
mod commands;
mod detect;
pub mod diagnostics;
pub(crate) mod editor;
mod error;
mod events;
mod library;
mod media;
mod media_server;
mod model;
mod runtime;
mod settings;
mod sharing;
mod shortcuts;
mod sound;
mod state;
mod tray;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::Manager;

use crate::capture::Recorder;
use crate::detect::Detector;
use crate::library::Library;
use crate::model::{RecorderState, Settings};
use crate::settings::SettingsStore;
use crate::state::AppState;

/// Repair environment differences introduced by the AppImage launcher. Call
/// before anything touches GStreamer (including `--doctor` and `--selftest`).
pub fn prepare_runtime_environment() {
    init_logging();
    crate::settings::migrate_legacy_dirs();
    runtime::startup_guard_begin();
    runtime::prepare();
}

/// Install the logger before any runtime repair, so `--doctor`, `--selftest`
/// and the AppImage messages reach the console.
fn init_logging() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .try_init();
}

/// Whether startup should ask for screen-capture permission: only when no
/// restore token is stored and nothing is already trying to capture. The
/// buffer itself is never started by the permission request.
fn should_request_permission(settings: &Settings, state: RecorderState) -> bool {
    let has_restore_token = settings
        .capture_monitor
        .as_ref()
        .and_then(|monitor| monitor.restore_token.as_ref())
        .is_some();
    !has_restore_token && state == RecorderState::Off
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    runtime::prepare();
    init_logging();

    let settings = SettingsStore::load();
    let recorder = Arc::new(Recorder::new(settings.clone()));
    let detector = Arc::new(Detector::new(settings.clone()));
    let library = Arc::new(Library::new());
    let media_server = match media_server::MediaServer::start(library.clone()) {
        Ok(server) => server,
        Err(err) => {
            log::error!("clip playback will be unavailable: {err}");
            media_server::MediaServer::unavailable()
        }
    };
    let media_base = media_server.base_url().to_string();

    let state = AppState {
        settings: settings.clone(),
        recorder: recorder.clone(),
        detector: detector.clone(),
        library,
        media_base,
        _media_server: media_server,
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None::<Vec<&str>>,
        ))
        .manage(state)
        .setup(move |app| {
            let handle = app.handle().clone();

            // The window is up; the next few seconds decide that this launch
            // was not a startup crash.
            let guard = handle.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                runtime::startup_guard_commit();
                if runtime::take_auto_compat_note() {
                    events::toast(
                        &guard,
                        "info",
                        "Compatibility rendering was turned on after repeated startup crashes. You can turn it off in Settings > App.",
                    );
                }
            });

            tray::init(&handle)?;

            if let Err(err) = shortcuts::init(&handle) {
                log::warn!("global hotkeys unavailable: {err}");
            }

            detector.spawn(handle.clone(), recorder.clone());

            if settings.get().start_buffer_on_launch {
                let recorder = recorder.clone();
                let handle = handle.clone();
                tauri::async_runtime::spawn(async move {
                    // A launch-time start is manual: game detection must never
                    // stop it when the game exits.
                    if let Err(err) = recorder.start(&handle).await {
                        log::warn!("could not start the replay buffer: {err}");
                        events::toast(
                            &handle,
                            "error",
                            format!("Could not start the replay buffer: {err}"),
                        );
                    }
                });
            }

            // Ask for screen-capture permission once the window exists, so the
            // portal picker has something to attach to. A stored restore token
            // means permission is already granted, so stay silent. This never
            // starts the buffer; it only records the monitor selection.
            let launch_settings = settings.clone();
            if should_request_permission(&launch_settings.get(), recorder.status().state) {
                let recorder = recorder.clone();
                let handle = handle.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(1200)).await;
                    // A launch-time start or an early Start may have asked for
                    // permission in the meantime; do not show the picker twice.
                    if !should_request_permission(
                        &launch_settings.get(),
                        recorder.status().state,
                    ) {
                        return;
                    }
                    match recorder.choose_monitor(&handle).await {
                        Ok(saved) => log::info!(
                            "screen capture permission granted ({}); the buffer stays idle",
                            saved.name
                        ),
                        Err(err) => {
                            if crate::capture::portal::is_cancelled(&err) {
                                log::info!("screen capture permission was not granted at launch");
                            } else {
                                log::warn!("could not request screen capture permission: {err}");
                            }
                            events::toast(
                                &handle,
                                "info",
                                "Screen capture permission not granted - Start will ask again.",
                            );
                        }
                    }
                });
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let minimize = window
                    .state::<AppState>()
                    .settings
                    .get()
                    .minimize_to_tray;
                if minimize {
                    api.prevent_close();
                    let _ = window.hide();

                    // Hiding a window that looks closed confuses first-time
                    // users; say once per run where it went and how to quit.
                    static TRAY_HINT_SHOWN: AtomicBool = AtomicBool::new(false);
                    if !TRAY_HINT_SHOWN.swap(true, Ordering::SeqCst) {
                        std::thread::spawn(|| {
                            let result = notify_rust::Notification::new()
                                .summary("Shmingus Clips is still running")
                                .body("It lives in the system tray. Alt+C saves a clip; quit from the tray menu.")
                                .icon("media-record")
                                .timeout(notify_rust::Timeout::Milliseconds(6000))
                                .show();
                            if let Err(err) = result {
                                log::warn!("could not show the tray notification: {err}");
                            }
                        });
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::save_settings,
            commands::get_media_base,
            commands::check_environment,
            commands::run_selftest,
            commands::get_audio_devices,
            commands::mic_level,
            commands::start_mic_level_monitor,
            commands::stop_mic_level_monitor,
            commands::get_recorder_status,
            commands::start_buffer,
            commands::stop_buffer,
            commands::toggle_mic,
            commands::choose_monitor,
            commands::save_clip,
            commands::get_detected_game,
            commands::list_clips,
            commands::delete_clip,
            commands::rename_clip,
            commands::set_clip_favorite,
            commands::trim_clip,
            commands::render_edit,
            commands::export_clip,
            commands::reveal_export,
            commands::open_clips_folder,
            commands::reveal_clip,
            commands::share_clip,
            commands::get_share,
            commands::list_shares,
            commands::delete_share,
            commands::test_share_server,
            commands::auth_status,
            commands::auth_login,
            commands::auth_logout,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Shmingus Clips")
        .run(|_app, event| {
            if let tauri::RunEvent::Exit = event {
                runtime::startup_guard_commit();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SavedMonitor;

    fn monitor(restore_token: Option<&str>) -> SavedMonitor {
        SavedMonitor {
            id: 1,
            name: "Monitor 1920×1080".into(),
            restore_token: restore_token.map(str::to_string),
        }
    }

    #[test]
    fn asks_for_permission_only_without_a_restore_token_and_while_idle() {
        let mut settings = Settings::default();
        assert!(should_request_permission(&settings, RecorderState::Off));

        // Already capturing (for example `start_buffer_on_launch`): loud is wrong.
        assert!(!should_request_permission(
            &settings,
            RecorderState::Starting
        ));
        assert!(!should_request_permission(
            &settings,
            RecorderState::Buffering
        ));

        // A restore token means the portal will not show the picker.
        settings.capture_monitor = Some(monitor(Some("token")));
        assert!(!should_request_permission(&settings, RecorderState::Off));

        // A saved monitor without a token still needs the permission prompt.
        settings.capture_monitor = Some(monitor(None));
        assert!(should_request_permission(&settings, RecorderState::Off));
    }
}
