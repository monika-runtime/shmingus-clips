//! Global hotkeys through the XDG GlobalShortcuts portal.
//!
//! Owned by the detection task. `init` probes the portal once (so `lib.rs` can
//! log the failure and keep running without hotkeys) and then creates a portal
//! session and binds the accelerators from the settings. `apply` re-reads the
//! hotkeys and replaces the bindings: the portal allows one `BindShortcuts` per
//! session, so the previous session is closed and a fresh one is bound instead
//! of leaving sessions behind.
//!
//! The first bind can show the desktop's shortcut approval dialog. KDE accepts
//! an empty parent window for this portal, and Tauri's Wayland window has no
//! xdg-foreign surface handle that ashpd 0.14 could turn into a
//! `WindowIdentifier` (that needs the crate's `gtk4`/`wayland` features, which
//! are not enabled), so no window identifier is passed.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ashpd::desktop::global_shortcuts::{BindShortcutsOptions, GlobalShortcuts, NewShortcut};
use ashpd::desktop::{CreateSessionOptions, Session};
use futures_util::StreamExt;
use tauri::{AppHandle, Manager};

use crate::error::{AppError, Result};
use crate::events;
use crate::model::{Hotkeys, RecorderState};
use crate::state::AppState;

/// Application-provided shortcut ids, as sent to the portal.
///
/// Namespaced because KDE records a shortcut id the first time it is bound: an
/// id it once stored with `current=none` (denied, or refused because something
/// else held the key) stays inactive forever, and every later bind for that id
/// comes back with an empty trigger. New ids start from a clean slate.
pub const SAVE_CLIP: &str = "net.shmingus.clips.save-clip";
pub const TOGGLE_BUFFER: &str = "net.shmingus.clips.toggle-buffer";
pub const TOGGLE_MIC: &str = "net.shmingus.clips.toggle-mic";

/// How long the start-up probe waits for the portal before giving up.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// What an activated shortcut does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    SaveClip,
    ToggleBuffer,
    ToggleMic,
}

impl Action {
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            SAVE_CLIP => Some(Self::SaveClip),
            TOGGLE_BUFFER => Some(Self::ToggleBuffer),
            TOGGLE_MIC => Some(Self::ToggleMic),
            _ => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::SaveClip => SAVE_CLIP,
            Self::ToggleBuffer => TOGGLE_BUFFER,
            Self::ToggleMic => TOGGLE_MIC,
        }
    }
}

/// The live portal session. Kept here so the portal keeps the shortcuts bound
/// for the whole process lifetime; a rebind replaces it (see [`apply`]).
struct Binding {
    portal: GlobalShortcuts,
    session: Session<GlobalShortcuts>,
}

static BINDING: OnceLock<tokio::sync::Mutex<Option<Binding>>> = OnceLock::new();
static DISPATCHER: AtomicBool = AtomicBool::new(false);

fn binding_slot() -> &'static tokio::sync::Mutex<Option<Binding>> {
    BINDING.get_or_init(|| tokio::sync::Mutex::new(None))
}

/// The portal descriptors for the three hotkeys in `hotkeys`. A blank
/// accelerator is sent without a preferred trigger.
pub fn shortcut_specs(hotkeys: &Hotkeys) -> Vec<NewShortcut> {
    [
        (SAVE_CLIP, "Save a clip", hotkeys.save_clip.as_str()),
        (
            TOGGLE_BUFFER,
            "Start or stop the replay buffer",
            hotkeys.toggle_buffer.as_str(),
        ),
        (
            TOGGLE_MIC,
            "Mute or unmute the microphone",
            hotkeys.toggle_mic.as_str(),
        ),
    ]
    .into_iter()
    .map(|(id, description, accelerator)| {
        NewShortcut::new(id, description).preferred_trigger(trigger(accelerator))
    })
    .collect()
}

fn trigger(accelerator: &str) -> Option<&str> {
    let trimmed = accelerator.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn portal_error(err: ashpd::Error) -> AppError {
    AppError::Portal(format!("GlobalShortcuts: {err}"))
}

/// Check the portal is there, then bind in the background.
///
/// The probe is synchronous because `lib.rs` logs an `Err` and carries on; it
/// runs on its own thread so a slow or missing portal can neither block nor
/// panic the main thread. Binding itself waits for the approval dialog and so
/// must not run here.
pub fn init(app: &AppHandle) -> Result<()> {
    probe_portal()?;

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        start_dispatcher(app.clone()).await;
        if let Err(err) = apply(&app).await {
            log::warn!("global hotkeys could not be bound: {err}");
            events::toast(
                &app,
                "error",
                format!("Global hotkeys could not be bound: {err}"),
            );
        }
    });
    Ok(())
}

fn probe_portal() -> Result<()> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("shmingus-clips-shortcuts-probe".into())
        .spawn(move || {
            let outcome = tauri::async_runtime::block_on(async {
                match tokio::time::timeout(PROBE_TIMEOUT, GlobalShortcuts::new()).await {
                    Ok(Ok(portal)) => Ok(portal.version()),
                    Ok(Err(err)) => Err(err.to_string()),
                    Err(_) => Err(format!("no answer after {}s", PROBE_TIMEOUT.as_secs())),
                }
            });
            let _ = sender.send(outcome);
        })
        .map_err(|err| AppError::Portal(format!("could not probe the portal: {err}")))?;

    match receiver.recv_timeout(PROBE_TIMEOUT + Duration::from_secs(2)) {
        Ok(Ok(version)) => {
            log::info!("GlobalShortcuts portal available (version {version})");
            Ok(())
        }
        Ok(Err(err)) => Err(AppError::Portal(format!(
            "the GlobalShortcuts portal is unavailable: {err}"
        ))),
        Err(_) => Err(AppError::Portal(
            "the GlobalShortcuts portal did not answer".into(),
        )),
    }
}

/// Listen for `Activated` once for the process lifetime. The signal is emitted
/// by the interface itself, not per session, so it survives a rebind.
async fn start_dispatcher(app: tauri::AppHandle) {
    if DISPATCHER.swap(true, Ordering::SeqCst) {
        return;
    }

    let portal = match GlobalShortcuts::new().await {
        Ok(portal) => portal,
        Err(err) => {
            DISPATCHER.store(false, Ordering::SeqCst);
            log::warn!("could not listen for global shortcuts: {err}");
            events::toast(
                &app,
                "error",
                format!("Global hotkeys are unavailable: {err}"),
            );
            return;
        }
    };
    let stream = match portal.receive_activated().await {
        Ok(stream) => stream,
        Err(err) => {
            DISPATCHER.store(false, Ordering::SeqCst);
            log::warn!("could not listen for global shortcuts: {err}");
            events::toast(
                &app,
                "error",
                format!("Global hotkeys are unavailable: {err}"),
            );
            return;
        }
    };

    tauri::async_runtime::spawn(async move {
        // The stream is `+ use<>` in ashpd 0.14, so it owns its connection and
        // keeping `portal` alive here is only for symmetry with future versions.
        let _portal = portal;
        let mut stream = stream;
        while let Some(activated) = stream.next().await {
            let Some(action) = Action::from_id(activated.shortcut_id()) else {
                log::debug!("unknown global shortcut '{}'", activated.shortcut_id());
                continue;
            };
            log::info!("global shortcut '{}' activated", action.id());
            let app = app.clone();
            // Saving a clip can take a moment; do not delay the next signal.
            tauri::async_runtime::spawn(async move { dispatch(action, app).await });
        }
        log::warn!("the GlobalShortcuts Activated signal stream ended");
        DISPATCHER.store(false, Ordering::SeqCst);
    });
}

/// Run the command behind a hotkey.
async fn dispatch(action: Action, app: tauri::AppHandle) {
    let state = app.state::<AppState>();
    match action {
        Action::SaveClip => {
            let game = state.detector.current();
            match state.recorder.save_clip(&app, game, None).await {
                Ok(clip) => events::toast(&app, "success", format!("Clip saved: {}", clip.title)),
                Err(err) => events::toast(&app, "error", format!("Could not save a clip: {err}")),
            }
        }
        Action::ToggleBuffer => {
            let buffering = matches!(
                state.recorder.status().state,
                RecorderState::Starting | RecorderState::Buffering
            );
            let result = if buffering {
                state.recorder.stop(&app).await.map(|_| ())
            } else {
                state.recorder.start(&app).await.map(|_| ())
            };
            if let Err(err) = result {
                events::toast(
                    &app,
                    "error",
                    format!("Could not toggle the replay buffer: {err}"),
                );
            }
        }
        Action::ToggleMic => {
            if let Err(err) = state.recorder.toggle_mic(&app).await {
                events::toast(
                    &app,
                    "error",
                    format!("Could not toggle the microphone: {err}"),
                );
            }
        }
    }
}

/// Re-register every binding from the current settings.
///
/// The portal permits one `BindShortcuts` per session ("An application can only
/// attempt bind shortcuts of a session once"), so rebinding means closing the
/// previous session and binding a new one. The lock serialises rebinds so
/// sessions cannot stack.
pub async fn apply(app: &AppHandle) -> Result<()> {
    let hotkeys = app.state::<AppState>().settings.get().hotkeys;

    let mut slot = binding_slot().lock().await;
    if let Some(previous) = slot.take() {
        if let Err(err) = previous.session.close().await {
            log::debug!("could not close the previous GlobalShortcuts session: {err}");
        } else {
            log::debug!("closed the previous GlobalShortcuts session");
        }
    }

    let binding = create_binding().await?;
    match binding.bind(&hotkeys).await {
        Ok(()) => {
            *slot = Some(binding);
            Ok(())
        }
        Err(err) => {
            // Do not keep a session around whose shortcuts were rejected: it
            // would block the next attempt from binding.
            if let Err(close_err) = binding.session.close().await {
                log::debug!("could not close the failed GlobalShortcuts session: {close_err}");
            }
            Err(err)
        }
    }
}

async fn create_binding() -> Result<Binding> {
    let portal = GlobalShortcuts::new().await.map_err(portal_error)?;
    let session = portal
        .create_session(CreateSessionOptions::default())
        .await
        .map_err(portal_error)?;
    log::info!("GlobalShortcuts session created");
    Ok(Binding { portal, session })
}

impl Binding {
    async fn bind(&self, hotkeys: &Hotkeys) -> Result<()> {
        let specs = shortcut_specs(hotkeys);
        // `None` = no parent window identifier; KDE accepts that for this portal.
        let request = self
            .portal
            .bind_shortcuts(&self.session, &specs, None, BindShortcutsOptions::default())
            .await
            .map_err(portal_error)?;
        // The request has already waited for the portal's Response signal, which
        // is where the approval dialog is answered.
        let reply = request.response().map_err(portal_error)?;
        let mut bound = 0usize;
        for shortcut in reply.shortcuts() {
            if shortcut.trigger_description().trim().is_empty() {
                log::warn!(
                    "global hotkey '{}' was not bound (the portal reported no trigger)",
                    shortcut.id()
                );
            } else {
                bound += 1;
            }
        }

        if bound == 0 {
            // The portal accepted the call but activated nothing. KDE does this
            // when the shortcuts are disabled for this app in System Settings,
            // so say what the user can do instead of failing silently.
            return Err(AppError::Portal(
                "the desktop did not activate any shortcut - enable Shmingus Clips under \
                 System Settings → Shortcuts, then save the hotkey settings again"
                    .into(),
            ));
        }

        log::info!(
            "global hotkeys bound ({bound}/3): save={}, buffer={}, mic={}",
            hotkeys.save_clip,
            hotkeys.toggle_buffer,
            hotkeys.toggle_mic
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_portal_ids_to_actions_and_back() {
        for action in [Action::SaveClip, Action::ToggleBuffer, Action::ToggleMic] {
            assert_eq!(Action::from_id(action.id()), Some(action));
        }
        assert_eq!(Action::from_id("unknown"), None);
        assert_eq!(Action::from_id(""), None);
    }

    #[test]
    fn every_hotkey_gets_a_portal_shortcut() {
        let specs = shortcut_specs(&Hotkeys::default());
        assert_eq!(specs.len(), 3);
        let debug = format!("{specs:?}");
        for wanted in [
            "save-clip",
            "ALT+c",
            "toggle-buffer",
            "CTRL+ALT+b",
            "toggle-mic",
            "CTRL+ALT+m",
        ] {
            assert!(debug.contains(wanted), "'{wanted}' missing from {debug}");
        }
    }

    #[test]
    fn trims_accelerators_and_drops_blank_ones() {
        assert_eq!(trigger("  CTRL+ALT+c "), Some("CTRL+ALT+c"));
        assert_eq!(trigger(""), None);
        assert_eq!(trigger("   "), None);

        let specs = shortcut_specs(&Hotkeys {
            save_clip: "   ".into(),
            ..Hotkeys::default()
        });
        assert!(!format!("{:?}", specs[0]).contains("preferred_trigger: Some"));
        assert!(format!("{:?}", specs[1]).contains("preferred_trigger: Some"));
    }
}
