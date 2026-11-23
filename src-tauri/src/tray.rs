use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::events;
use crate::model::RecorderState;
use crate::state::AppState;

/// Tray icon: open the window, save a clip, toggle the buffer, open the clips
/// folder, quit. Left click brings the main window up.
pub fn init(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Shmingus Clips", true, None::<&str>)?;
    let save = MenuItem::with_id(app, "save_clip", "Save clip", true, None::<&str>)?;
    let toggle = MenuItem::with_id(
        app,
        "toggle_buffer",
        "Start / stop replay buffer",
        true,
        None::<&str>,
    )?;
    let folder = MenuItem::with_id(app, "open_folder", "Open clips folder", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Shmingus Clips", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;

    let items: [&dyn tauri::menu::IsMenuItem<tauri::Wry>; 7] =
        [&open, &save, &toggle, &sep1, &folder, &sep2, &quit];
    let menu = Menu::with_items(app, &items)?;

    let mut builder = TrayIconBuilder::with_id("shmingus-clips-tray")
        .menu(&menu)
        .tooltip("Shmingus Clips - replay buffer")
        .show_menu_on_left_click(false);
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }

    builder
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_window(tray.app_handle());
            }
        })
        .on_menu_event(|app, event| {
            let id: &str = event.id().as_ref();
            match id {
                "open" => show_window(app),
                "save_clip" => save_clip(app),
                "toggle_buffer" => toggle_buffer(app),
                "open_folder" => open_folder(app),
                "quit" => app.exit(0),
                _ => {}
            }
        })
        .build(app)?;

    Ok(())
}

fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn save_clip(app: &AppHandle) {
    let state = app.state::<AppState>();
    let recorder = state.recorder.clone();
    let detector = state.detector.clone();
    let handle = app.clone();

    tauri::async_runtime::spawn(async move {
        match recorder.save_clip(&handle, detector.current(), None).await {
            Ok(clip) => {
                events::toast(&handle, "success", format!("Clip saved - {}", clip.title));
            }
            Err(err) => {
                events::toast(&handle, "error", format!("Could not save clip: {err}"));
            }
        }
    });
}

fn toggle_buffer(app: &AppHandle) {
    let state = app.state::<AppState>();
    let recorder = state.recorder.clone();
    let handle = app.clone();

    tauri::async_runtime::spawn(async move {
        let running = recorder.status().state == RecorderState::Buffering;
        let result = if running {
            recorder.stop(&handle).await
        } else {
            recorder.start(&handle).await
        };
        if let Err(err) = result {
            events::toast(&handle, "error", format!("Recorder: {err}"));
        }
    });
}

fn open_folder(app: &AppHandle) {
    let dir = app.state::<AppState>().settings.get().save_dir;
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
}
