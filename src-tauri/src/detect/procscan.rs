//! Scanning a `/proc`-shaped directory for running games.
//!
//! Everything here takes the proc root as a parameter so the scanner can run
//! against a fixture directory in tests. Entries disappearing between listing
//! and reading a file are expected (processes exit); every read is fallible and
//! skipped, never panicked on.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::model::{GameInfo, GameSource};

/// A running process that looks like a game, with the data needed to pick
/// between several of them.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub pid: u32,
    /// `/proc/<pid>/stat` field 22, in clock ticks since boot. Orders launches.
    pub start_time: u64,
    pub info: GameInfo,
}

/// Environment variables Steam exports into the processes of a launched game,
/// most specific first.
const STEAM_ID_VARS: [&str; 3] = ["SteamAppId", "SteamGameId", "SteamOverlayGameId"];

/// Process names that are part of a launcher or a runtime rather than a game.
/// Compared case-insensitively as prefixes, e.g. `wine64-preloader` and
/// `steamwebhelper` are both launcher processes.
const IGNORED_PREFIXES: [&str; 11] = [
    "steam",
    "wine",
    "proton",
    "pressure-vessel",
    "pv-bwrap",
    "reaper",
    "bwrap",
    "gamescope",
    "heroic",
    "lutris",
    "shmingus-clips",
];

/// Parse a NUL-separated `/proc/<pid>/environ` block.
pub fn parse_environ(block: &[u8]) -> HashMap<String, String> {
    block
        .split(|byte| *byte == 0)
        .filter_map(|entry| {
            let text = std::str::from_utf8(entry).ok()?;
            let (key, value) = text.split_once('=')?;
            if key.is_empty() {
                return None;
            }
            Some((key.to_string(), value.to_string()))
        })
        .collect()
}

/// PIDs listed in `<proc_root>`, skipping anything that is not a numeric
/// directory (kernel threads have names like `kworker/0:1`, `self` is a link).
pub fn list_pids(proc_root: &Path) -> Vec<u32> {
    let Ok(entries) = std::fs::read_dir(proc_root) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name();
            let pid = name.to_str()?.parse::<u32>().ok()?;
            Some(pid)
        })
        .collect()
}

fn read_file(proc_root: &Path, pid: u32, name: &str) -> Option<Vec<u8>> {
    std::fs::read(proc_root.join(pid.to_string()).join(name)).ok()
}

pub fn read_environ(proc_root: &Path, pid: u32) -> Option<HashMap<String, String>> {
    let block = read_file(proc_root, pid, "environ")?;
    Some(parse_environ(&block))
}

pub fn read_comm(proc_root: &Path, pid: u32) -> Option<String> {
    let bytes = read_file(proc_root, pid, "comm")?;
    let text = String::from_utf8_lossy(&bytes);
    Some(text.trim_end_matches('\n').to_string())
}

/// The program a process runs. Fails for kernel threads and other users'
/// processes, which is why every caller treats it as best effort.
pub fn read_exe(proc_root: &Path, pid: u32) -> Option<PathBuf> {
    std::fs::read_link(proc_root.join(pid.to_string()).join("exe")).ok()
}

/// Field 22 of `/proc/<pid>/stat`: the process start time. The `comm` field is
/// wrapped in parentheses and may itself contain spaces or parentheses, so it
/// is stripped from the last `)` before splitting the numeric fields.
pub fn parse_start_time(stat: &str) -> Option<u64> {
    let rest = stat.get(stat.rfind(')')? + 1..)?;
    rest.split_whitespace().nth(19)?.parse().ok()
}

/// Stat file contents split so tests can build one, or `None` when the process
/// vanished mid-scan.
pub fn read_start_time(proc_root: &Path, pid: u32) -> Option<u64> {
    let bytes = read_file(proc_root, pid, "stat")?;
    parse_start_time(&String::from_utf8_lossy(&bytes))
}

/// The Steam app id exported into `env`, if any. `0` and non-numeric values
/// (Steam sets `SteamAppId=0` for its own helpers) are rejected.
pub fn steam_app_id(env: &HashMap<String, String>) -> Option<u32> {
    for key in STEAM_ID_VARS {
        let Some(value) = env.get(key) else { continue };
        let Ok(id) = value.trim().parse::<u32>() else {
            continue;
        };
        if id > 0 {
            return Some(id);
        }
    }
    None
}

/// Title Heroic exports to a launched game (verified against the Heroic 2.x
/// launch scripts on this machine: `HEROIC_GAME_TITLE` from the launch script,
/// `HEROIC_APP_NAME` from the umu/wine wrapper). Lutris is not installed here,
/// so it is deliberately not guessed at.
pub fn heroic_title(env: &HashMap<String, String>) -> Option<String> {
    for key in ["HEROIC_GAME_TITLE", "HEROIC_APP_NAME"] {
        let Some(value) = env.get(key).map(|v| v.trim()) else {
            continue;
        };
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

/// Whether an environment is worth reading further.
pub fn looks_like_game(env: &HashMap<String, String>) -> bool {
    steam_app_id(env).is_some() || heroic_title(env).is_some()
}

fn is_ignored_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower.is_empty() {
        return false;
    }
    IGNORED_PREFIXES
        .iter()
        .any(|prefix| lower.starts_with(prefix))
}

/// Launchers, runtimes and Shmingus Clips itself inherit the same environment as the
/// game (Proton even reuses `SteamAppId`), so they must not be reported as the
/// running game.
pub fn is_ignored_process(comm: &str, exe: Option<&Path>) -> bool {
    if is_ignored_name(comm) {
        return true;
    }
    if let Some(exe) = exe {
        if let Some(file) = exe.file_name().and_then(|f| f.to_str()) {
            if is_ignored_name(file) {
                return true;
            }
        }
        if let Some(dir) = exe.to_str() {
            let lower = dir.to_ascii_lowercase();
            if lower.contains("/shmingus-clips/") {
                return true;
            }
        }
    }
    false
}

/// Turn the environment of one process into the game it belongs to.
pub fn candidate_from_env(pid: u32, env: &HashMap<String, String>) -> Option<GameInfo> {
    if let Some(app_id) = steam_app_id(env) {
        return Some(GameInfo {
            id: format!("steam:{app_id}"),
            // Replaced by the manifest name in `resolve_steam_names`.
            name: format!("Steam App {app_id}"),
            source: GameSource::Steam,
            pid: Some(pid),
        });
    }
    if let Some(title) = heroic_title(env) {
        let app = env
            .get("HEROIC_GAME_APP_NAME")
            .map(|app| app.trim())
            .filter(|app| !app.is_empty())
            .unwrap_or(title.as_str());
        return Some(GameInfo {
            id: format!("heroic:{app}"),
            name: title,
            source: GameSource::Heroic,
            pid: Some(pid),
        });
    }
    None
}

/// Every process under `proc_root` that belongs to a game.
pub fn scan(proc_root: &Path) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    for pid in list_pids(proc_root) {
        // A process may exit between `read_dir` and this read: skip it.
        let Some(env) = read_environ(proc_root, pid) else {
            continue;
        };
        if !looks_like_game(&env) {
            continue;
        }
        let comm = read_comm(proc_root, pid).unwrap_or_default();
        let exe = read_exe(proc_root, pid);
        if is_ignored_process(&comm, exe.as_deref()) {
            continue;
        }
        let Some(info) = candidate_from_env(pid, &env) else {
            continue;
        };
        let start_time = read_start_time(proc_root, pid).unwrap_or(0);
        candidates.push(Candidate {
            pid,
            start_time,
            info,
        });
    }
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::test_support::TempDir;

    fn env_of(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn parses_nul_separated_environ() {
        let block = b"PATH=/usr/bin\0SteamAppId=570\0BADENTRY\0=nameless\0";
        let env = parse_environ(block);
        assert_eq!(env.get("PATH").map(String::as_str), Some("/usr/bin"));
        assert_eq!(env.get("SteamAppId").map(String::as_str), Some("570"));
        assert_eq!(env.len(), 2);
    }

    #[test]
    fn steam_id_prefers_the_most_specific_variable() {
        let env = env_of(&[
            ("SteamOverlayGameId", "999"),
            ("SteamGameId", "570"),
            ("SteamAppId", "570"),
        ]);
        assert_eq!(steam_app_id(&env), Some(570));

        let overlay_only = env_of(&[("SteamOverlayGameId", "440")]);
        assert_eq!(steam_app_id(&overlay_only), Some(440));
    }

    #[test]
    fn steam_id_rejects_launcher_and_garbage_values() {
        assert_eq!(steam_app_id(&env_of(&[("SteamAppId", "0")])), None);
        assert_eq!(steam_app_id(&env_of(&[("SteamAppId", "nonsteam")])), None);
        assert_eq!(steam_app_id(&env_of(&[])), None);
    }

    #[test]
    fn builds_a_steam_candidate() {
        let info = candidate_from_env(42, &env_of(&[("SteamAppId", "570")])).expect("candidate");
        assert_eq!(info.id, "steam:570");
        assert_eq!(info.source, GameSource::Steam);
        assert_eq!(info.pid, Some(42));
        assert_eq!(info.name, "Steam App 570");
    }

    #[test]
    fn builds_a_heroic_candidate_from_the_launch_script_env() {
        let info = candidate_from_env(
            7,
            &env_of(&[
                ("HEROIC_GAME_TITLE", "Celeste"),
                ("HEROIC_GAME_APP_NAME", "celeste-gog"),
            ]),
        )
        .expect("candidate");
        assert_eq!(info.id, "heroic:celeste-gog");
        assert_eq!(info.name, "Celeste");
        assert_eq!(info.source, GameSource::Heroic);
    }

    #[test]
    fn plain_environments_are_not_games() {
        assert!(!looks_like_game(&env_of(&[("PATH", "/usr/bin")])));
        assert!(candidate_from_env(1, &env_of(&[("PATH", "/usr/bin")])).is_none());
    }

    #[test]
    fn launchers_and_runtimes_are_ignored() {
        for comm in [
            "steam",
            "steamwebhelper",
            "wine64-preloader",
            "wineserver",
            "proton",
            "reaper",
            "pressure-vessel",
            "heroic",
            "lutris",
            "shmingus-clips",
        ] {
            assert!(is_ignored_process(comm, None), "{comm} should be ignored");
        }
        assert!(!is_ignored_process("Celeste.exe", None));
        assert!(is_ignored_process(
            "something",
            Some(Path::new(
                "/home/user/shmingus-clips/src-tauri/target/debug/shmingus-clips"
            ))
        ));
    }

    #[test]
    fn start_time_ignores_braces_and_spaces_in_comm() {
        // Fields after `)` are 3..22; start time is the 20th of them.
        let stat =
            "4242 (My Game (beta)) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 180000 20 21";
        assert_eq!(parse_start_time(stat), Some(180_000));
        assert_eq!(parse_start_time("garbage"), None);
    }

    fn write(path: &Path, contents: &[u8]) {
        std::fs::write(path, contents).expect("write fixture");
    }

    fn fake_process(root: &Path, pid: u32, comm: &str, start_time: u64, env: &[(&str, &str)]) {
        let dir = root.join(pid.to_string());
        std::fs::create_dir_all(&dir).expect("proc dir");
        let environ: Vec<u8> = env
            .iter()
            .flat_map(|(key, value)| format!("{key}={value}\0").into_bytes())
            .collect();
        write(&dir.join("environ"), &environ);
        write(&dir.join("comm"), format!("{comm}\n").as_bytes());
        // 18 filler numeric fields after `state`, then the start time (field 22).
        let stat = format!("{pid} ({comm}) S {} {start_time} 0 0", "1 ".repeat(18));
        write(&dir.join("stat"), stat.as_bytes());
    }

    #[test]
    fn scans_a_fixture_proc_tree() {
        let root = TempDir::new("scan");
        fake_process(&root.path(), 100, "Game.exe", 500, &[("SteamAppId", "570")]);
        fake_process(
            &root.path(),
            101,
            "Celeste.exe",
            700,
            &[("HEROIC_GAME_TITLE", "Celeste")],
        );
        fake_process(
            &root.path(),
            102,
            "wineserver",
            900,
            &[("SteamAppId", "570")],
        );
        // A numeric directory with no readable files: a process that vanished.
        std::fs::create_dir_all(root.path().join("103")).expect("empty pid dir");
        // A non-numeric entry such as `self` or a kernel thread name.
        std::fs::create_dir_all(root.path().join("self")).ok();

        let mut candidates = scan(&root.path());
        candidates.sort_by_key(|c| c.pid);
        assert_eq!(
            candidates.iter().map(|c| c.pid).collect::<Vec<_>>(),
            vec![100, 101]
        );
        assert_eq!(candidates[0].start_time, 500);
        assert_eq!(candidates[1].info.name, "Celeste");
    }

    #[test]
    fn scan_tolerates_a_missing_proc_root() {
        let root = TempDir::new("missing");
        assert!(scan(&root.path().join("does-not-exist")).is_empty());
    }
}
