//! Game detection: which game is running right now.
//!
//! Owned by the detection task. A background loop polls a `/proc`-shaped
//! directory every few seconds, keeps the most plausible game, and emits
//! `game:detected` only when the answer changes. Steam is the main source;
//! Heroic is best effort. Lutris is not installed on this machine, so no
//! Lutris-specific environment is guessed at (that would risk reporting a
//! launcher as a game).

mod procscan;
mod steam;

#[cfg(test)]
mod test_support;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::RwLock;
use tauri::AppHandle;

use crate::capture::Recorder;
use crate::events;
use crate::model::{GameInfo, GameSource};
use crate::settings::SettingsStore;

/// How often `/proc` is scanned. Detection is cheap (a few hundred small files)
/// and a game showing up within three seconds is fast enough for clip naming.
const POLL_INTERVAL: Duration = Duration::from_secs(3);

/// What a game change should do to the replay buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureTrigger {
    None,
    Start,
    Stop,
}

pub struct Detector {
    settings: Arc<SettingsStore>,
    current: RwLock<Option<GameInfo>>,
    running: AtomicBool,
}

impl Detector {
    pub fn new(settings: Arc<SettingsStore>) -> Self {
        Self {
            settings,
            current: RwLock::new(None),
            running: AtomicBool::new(false),
        }
    }

    pub fn current(&self) -> Option<GameInfo> {
        self.current.read().clone()
    }

    /// Start the background polling loop. Emits `game:detected` on change and
    /// starts/stops the replay buffer when the setting asks for it.
    pub fn spawn(self: &Arc<Self>, app: AppHandle, recorder: Arc<Recorder>) {
        if self.running.swap(true, Ordering::SeqCst) {
            log::debug!("game detection is already running");
            return;
        }

        let detector = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            let libraries = steam::SteamLibraries::new();
            log::info!(
                "game detection started ({} Steam steamapps directories)",
                libraries.steamapps_dirs().len()
            );
            loop {
                let enabled = detector.settings.get().auto_detect_games;
                let previous = detector.current();
                let next = detect_in(Path::new("/proc"), &libraries, enabled, previous.as_ref());
                // Only a change drives the capture trigger: a manual Stop while
                // a game is still running must not be undone by the next poll.
                if detector.publish(&app, next.clone()) {
                    detector.apply_capture_trigger(&app, &recorder, enabled, previous, next);
                }
                tokio::time::sleep(POLL_INTERVAL).await;
            }
        });
    }

    /// React to a detected game change without blocking the polling loop.
    fn apply_capture_trigger(
        &self,
        app: &AppHandle,
        recorder: &Arc<Recorder>,
        detection_enabled: bool,
        previous: Option<GameInfo>,
        next: Option<GameInfo>,
    ) {
        match capture_trigger(
            detection_enabled,
            self.settings.get().auto_start_on_game,
            next.as_ref(),
        ) {
            CaptureTrigger::None => {}
            CaptureTrigger::Start => {
                if let Some(game) = next.as_ref() {
                    log::info!("{} detected; starting the replay buffer", game.name);
                }
                let app = app.clone();
                let recorder = Arc::clone(recorder);
                tauri::async_runtime::spawn(async move {
                    if let Err(err) = recorder.start_auto(&app).await {
                        // The recorder shows a toast for real failures and
                        // stays quiet when the screen picker was dismissed.
                        log::warn!("automatic replay-buffer start failed: {err}");
                    }
                });
            }
            CaptureTrigger::Stop => {
                let save_first = self.settings.get().save_on_game_exit;
                match previous.as_ref() {
                    Some(game) if save_first => log::info!(
                        "{} exited; saving the buffered moments before stopping",
                        game.name
                    ),
                    Some(game) => log::info!("{} exited; stopping the replay buffer", game.name),
                    None => {}
                }
                let app = app.clone();
                let recorder = Arc::clone(recorder);
                tauri::async_runtime::spawn(async move {
                    let result = if save_first {
                        recorder
                            .save_and_stop_auto(&app, previous)
                            .await
                            .map(|_| ())
                    } else {
                        recorder.stop_auto(&app).await.map(|_| ())
                    };
                    if let Err(err) = result {
                        log::warn!("automatic replay-buffer stop failed: {err}");
                    }
                });
            }
        }
    }

    /// Replace the current game and tell the frontend, but only on a change.
    ///
    /// The process id is deliberately not part of the comparison: one game
    /// normally owns several processes, and which of them the scan reports can
    /// change from poll to poll. Only a different game counts as a change.
    ///
    /// Returns whether this was a change the capture trigger should act on.
    fn publish(&self, app: &AppHandle, next: Option<GameInfo>) -> bool {
        {
            let mut current = self.current.write();
            if same_game(current.as_ref(), next.as_ref()) {
                // Same game: refresh the pid without bothering the frontend.
                *current = next;
                return false;
            }
            *current = next.clone();
        }
        match &next {
            Some(game) => log::info!(
                "detected {} ({:?}, pid {:?})",
                game.name,
                game.source,
                game.pid
            ),
            None => log::info!("no game detected"),
        }
        events::game_detected(app, next);
        true
    }
}

/// What a game change should do to the capture session.
///
/// A game exit always asks for a stop - `stop_auto` itself decides whether the
/// session was auto-started, so a session started automatically before the
/// setting was turned off is still stopped on the next exit. A game appearing
/// only starts the buffer while `autoStartOnGame` is on. When detection is
/// disabled the empty answer is not a game exit, so nothing is touched.
fn capture_trigger(
    detection_enabled: bool,
    auto_start_on_game: bool,
    next: Option<&GameInfo>,
) -> CaptureTrigger {
    if !detection_enabled {
        return CaptureTrigger::None;
    }
    match next {
        Some(_) if auto_start_on_game => CaptureTrigger::Start,
        Some(_) => CaptureTrigger::None,
        None => CaptureTrigger::Stop,
    }
}

/// Whether two (possibly absent) detections name the same game.
fn same_game(a: Option<&GameInfo>, b: Option<&GameInfo>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.id == b.id && a.name == b.name && a.source == b.source,
        _ => false,
    }
}

/// One detection pass. `enabled` mirrors `settings.auto_detect_games`: when the
/// user turns detection off the answer clears to `None` without scanning.
fn detect_in(
    proc_root: &Path,
    libraries: &steam::SteamLibraries,
    enabled: bool,
    previous: Option<&GameInfo>,
) -> Option<GameInfo> {
    if !enabled {
        return None;
    }
    let mut candidates = procscan::scan(proc_root);
    resolve_steam_names(&mut candidates, libraries);
    select(&candidates, previous)
}

/// Swap the provisional `Steam App <id>` name for the manifest name.
fn resolve_steam_names(candidates: &mut [procscan::Candidate], libraries: &steam::SteamLibraries) {
    for candidate in candidates {
        if candidate.info.source != GameSource::Steam {
            continue;
        }
        if let Some(app_id) = steam::app_id_from_game_id(&candidate.info.id) {
            candidate.info.name = libraries.display_name(app_id);
        }
    }
}

/// Pick the game to report: the previous one while it is still running,
/// otherwise the most recently started candidate. `None` when nothing runs.
fn select(candidates: &[procscan::Candidate], previous: Option<&GameInfo>) -> Option<GameInfo> {
    if let Some(previous) = previous {
        if let Some(found) = candidates.iter().find(|c| c.info.id == previous.id) {
            return Some(found.info.clone());
        }
    }
    candidates
        .iter()
        .max_by(|a, b| {
            a.start_time
                .cmp(&b.start_time)
                .then_with(|| b.pid.cmp(&a.pid))
        })
        .map(|candidate| candidate.info.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::test_support::TempDir;
    use crate::model::GameSource;

    fn candidate(pid: u32, start_time: u64, id: &str) -> procscan::Candidate {
        procscan::Candidate {
            pid,
            start_time,
            info: GameInfo {
                id: id.to_string(),
                name: id.to_string(),
                source: GameSource::Steam,
                pid: Some(pid),
            },
        }
    }

    fn game(id: &str) -> GameInfo {
        GameInfo {
            id: id.to_string(),
            name: id.to_string(),
            source: GameSource::Steam,
            pid: None,
        }
    }

    #[test]
    fn picks_the_most_recently_started_game() {
        let candidates = vec![
            candidate(10, 100, "steam:570"),
            candidate(11, 900, "steam:730"),
            candidate(12, 500, "steam:440"),
        ];
        assert_eq!(
            select(&candidates, None).map(|g| g.id),
            Some("steam:730".into())
        );
    }

    #[test]
    fn keeps_the_current_game_while_it_is_running() {
        let previous = game("steam:570");
        let candidates = vec![
            candidate(10, 100, "steam:570"),
            candidate(11, 900, "steam:730"),
        ];
        // A newer process must not steal the detection from a game still running.
        assert_eq!(
            select(&candidates, Some(&previous)).map(|g| g.id),
            Some("steam:570".into())
        );
    }

    #[test]
    fn switches_when_the_current_game_exits() {
        let previous = game("steam:570");
        let candidates = vec![candidate(11, 900, "steam:730")];
        assert_eq!(
            select(&candidates, Some(&previous)).map(|g| g.id),
            Some("steam:730".into())
        );
    }

    #[test]
    fn clears_when_nothing_runs() {
        assert!(select(&[], None).is_none());
        assert!(select(&[], Some(&game("steam:570"))).is_none());
    }

    #[test]
    fn a_changed_pid_is_not_a_changed_game() {
        let mut first = game("steam:570");
        first.pid = Some(10);
        let mut second = game("steam:570");
        second.pid = Some(11);
        assert!(same_game(Some(&first), Some(&second)));
        assert!(!same_game(Some(&first), Some(&game("steam:730"))));
        assert!(!same_game(Some(&first), None));
        assert!(same_game(None, None));
    }

    #[test]
    fn a_resolved_name_counts_as_a_change() {
        let mut placeholder = game("steam:1091500");
        placeholder.name = "Steam App 1091500".into();
        let mut resolved = game("steam:1091500");
        resolved.name = "Cyberpunk 2077".into();
        assert!(!same_game(Some(&placeholder), Some(&resolved)));
    }

    #[test]
    fn a_disabled_setting_stops_detection() {
        let root = TempDir::new("disabled");
        let dir = root.path().join("200");
        std::fs::create_dir_all(&dir).expect("pid dir");
        std::fs::write(dir.join("environ"), b"SteamAppId=570\0").expect("environ");
        let libraries = steam::SteamLibraries::from_roots(Vec::new());

        assert_eq!(
            detect_in(&root.path(), &libraries, true, None).map(|g| g.id),
            Some("steam:570".into())
        );
        assert!(detect_in(&root.path(), &libraries, false, None).is_none());
    }

    #[test]
    fn a_detected_game_starts_capture_when_the_setting_is_on() {
        let game = game("steam:570");
        assert_eq!(
            capture_trigger(true, true, Some(&game)),
            CaptureTrigger::Start
        );
        assert_eq!(
            capture_trigger(true, false, Some(&game)),
            CaptureTrigger::None
        );
    }

    #[test]
    fn a_game_exit_stops_capture_even_after_the_setting_was_turned_off() {
        // The setting only gates new automatic starts; `stop_auto` decides
        // whether the running session was auto-started.
        assert_eq!(capture_trigger(true, true, None), CaptureTrigger::Stop);
        assert_eq!(capture_trigger(true, false, None), CaptureTrigger::Stop);
    }

    #[test]
    fn disabling_detection_does_not_touch_the_buffer() {
        let game = game("steam:570");
        assert_eq!(capture_trigger(false, true, None), CaptureTrigger::None);
        assert_eq!(
            capture_trigger(false, true, Some(&game)),
            CaptureTrigger::None
        );
    }

    #[test]
    fn fixture_proc_tree_resolves_the_manifest_name() {
        let proc_root = TempDir::new("pipeline-proc");
        let steam_root = TempDir::new("pipeline-steam");
        std::fs::create_dir_all(steam_root.path().join("steamapps")).expect("steamapps");
        std::fs::write(
            steam_root.path().join("steamapps/appmanifest_1091500.acf"),
            "\"AppState\"\n{\n\t\"appid\"\t\t\"1091500\"\n\t\"name\"\t\t\"Cyberpunk 2077\"\n}\n",
        )
        .expect("manifest");

        let pid_dir = proc_root.path().join("300");
        std::fs::create_dir_all(&pid_dir).expect("pid dir");
        std::fs::write(
            pid_dir.join("environ"),
            b"PATH=/usr/bin\0SteamAppId=1091500\0",
        )
        .expect("environ");
        std::fs::write(pid_dir.join("comm"), b"Cyberpunk2077.exe\n").expect("comm");
        let stat = format!("300 (Cyberpunk2077.exe) S {} 4000 0 0", "1 ".repeat(18));
        std::fs::write(pid_dir.join("stat"), stat).expect("stat");

        let libraries = steam::SteamLibraries::from_roots(vec![steam_root.path().to_path_buf()]);
        let detected = detect_in(&proc_root.path(), &libraries, true, None).expect("a game");
        assert_eq!(detected.name, "Cyberpunk 2077");
        assert_eq!(detected.source, GameSource::Steam);
        assert_eq!(detected.pid, Some(300));
    }
}
