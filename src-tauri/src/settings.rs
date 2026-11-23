use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::RwLock;

use crate::error::Result;
use crate::model::Settings;

/// JSON-backed settings store at `~/.config/shmingus-clips/settings.json`.
pub struct SettingsStore {
    path: PathBuf,
    inner: RwLock<Settings>,
}

impl SettingsStore {
    pub fn load() -> Arc<Self> {
        migrate_legacy_dirs();
        let path = config_dir().join("settings.json");
        let settings = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Settings>(&raw).ok())
            .map(Settings::migrate)
            .map(Settings::normalized)
            .unwrap_or_else(|| Settings::default().normalized());

        let _ = ensure_dir(&settings.save_dir);

        Arc::new(Self {
            path,
            inner: RwLock::new(settings),
        })
    }

    pub fn get(&self) -> Settings {
        self.inner.read().clone()
    }

    pub fn replace(&self, next: Settings) -> Result<Settings> {
        let next = next.normalized();
        ensure_dir(&next.save_dir)?;

        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(&next)?;
        std::fs::write(&self.path, json)?;

        *self.inner.write() = next.clone();
        Ok(next)
    }
}

impl Settings {
    /// Move settings written by an older build onto today's defaults.
    pub fn migrate(mut self) -> Self {
        // The save hotkey used to be CTRL+ALT+c, which KDE did not deliver;
        // every settings file from that build carries exactly the old default.
        if self.hotkeys.save_clip == "CTRL+ALT+c" {
            self.hotkeys.save_clip = "ALT+c".into();
        }
        self
    }

    /// Clamp values into safe ranges. Called on load and before every save.
    pub fn normalized(mut self) -> Self {
        self.buffer_seconds = self.buffer_seconds.clamp(5, 300);
        self.fps = self.fps.clamp(15, 120);
        self.bitrate_mbps = self.bitrate_mbps.clamp(2, 100);
        self.max_width = self.max_width.clamp(0, 7680);
        self.audio.desktop_volume = self.audio.desktop_volume.clamp(0.0, 2.0);
        self.audio.mic_volume = self.audio.mic_volume.clamp(0.0, 2.0);
        if self.save_dir.trim().is_empty() {
            self.save_dir = crate::model::default_save_dir();
        }
        self
    }
}

/// Move `~/.config/clipforge` and `~/.cache/clipforge` (the pre-rename names) to
/// their Shmingus Clips paths, leaving a symlink behind so absolute thumbnail
/// paths stored in the library index keep resolving.
pub fn migrate_legacy_dirs() {
    migrate_dir(&legacy_config_dir(), &config_dir());
    migrate_dir(&legacy_cache_dir(), &cache_dir());
}

fn legacy_config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("clipforge")
}

fn legacy_cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| std::env::temp_dir())
        .join("clipforge")
}

fn migrate_dir(old: &Path, new: &Path) {
    if new.exists() || !old.exists() || std::fs::rename(old, new).is_err() {
        return;
    }
    #[cfg(unix)]
    {
        let _ = std::os::unix::fs::symlink(new, old);
    }
}

/// Whether the interface must be drawn without GPU acceleration. Read before
/// the webview starts, from the settings file (no store yet) or the
/// `SHMINGUS_CLIPS_SOFTWARE_RENDERING` environment override.
pub fn software_rendering_requested() -> bool {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let env = std::env::var("SHMINGUS_CLIPS_SOFTWARE_RENDERING").ok();
    let path = config_dir().join("settings.json");
    let stored = std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Settings>(&raw).ok())
        .map(|settings| settings.software_rendering)
        .unwrap_or(false);
    render_choice(&args, env.as_deref(), stored)
}

/// Flags beat the environment, which beats the saved setting. The flags exist
/// so a bad choice can always be undone from a terminal, even when the window
/// never appears.
fn render_choice(args: &[String], env: Option<&str>, stored: bool) -> bool {
    if args.iter().any(|arg| arg == "--no-software-rendering") {
        return false;
    }
    if args.iter().any(|arg| arg == "--software-rendering") {
        return true;
    }
    if let Some(value) = env {
        let value = value.trim().to_ascii_lowercase();
        return !["0", "false", "off", "no"].contains(&value.as_str());
    }
    stored
}

/// Turn Compatibility rendering on in the saved settings. Used after the app
/// detects repeated startup crashes, so the choice is visible in Settings.
pub fn enable_software_rendering() -> Result<()> {
    let path = config_dir().join("settings.json");
    let mut settings = std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Settings>(&raw).ok())
        .unwrap_or_default();
    if settings.software_rendering {
        return Ok(());
    }
    settings.software_rendering = true;
    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;
    let json = serde_json::to_string_pretty(&settings)?;
    std::fs::write(path, json)?;
    Ok(())
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("shmingus-clips")
}

pub fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| std::env::temp_dir())
        .join("shmingus-clips")
}

pub fn ensure_dir(path: impl AsRef<Path>) -> Result<PathBuf> {
    let path = path.as_ref().to_path_buf();
    std::fs::create_dir_all(&path)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_choice_prefers_flags_then_environment_then_settings() {
        let none: Vec<String> = Vec::new();
        assert!(super::render_choice(&none, None, true));
        assert!(!super::render_choice(&none, None, false));
        assert!(super::render_choice(&none, Some("1"), false));
        assert!(!super::render_choice(&none, Some("0"), true));
        assert!(!super::render_choice(&none, Some("false"), true));
        let off = vec!["--no-software-rendering".to_string()];
        assert!(!super::render_choice(&off, Some("1"), true));
        let on = vec!["--software-rendering".to_string()];
        assert!(super::render_choice(&on, Some("0"), false));
    }

    #[test]
    fn auto_start_on_game_defaults_to_true() {
        assert!(Settings::default().auto_start_on_game);
        assert!(Settings::default().normalized().auto_start_on_game);
    }

    #[test]
    fn an_older_settings_file_keeps_auto_start_on_game_on() {
        // Written before `autoStartOnGame` existed: the container-level
        // `serde(default)` fills it from `Settings::default()`.
        let legacy = r#"{
            "saveDir": "/tmp/shmingus-clips",
            "bufferSeconds": 30,
            "fps": 60,
            "bitrateMbps": 20,
            "encoder": "auto",
            "captureMonitor": null,
            "autoDetectGames": true,
            "startBufferOnLaunch": false,
            "minimizeToTray": true,
            "launchAtLogin": false,
            "notifyOnClip": true,
            "maxWidth": 1920
        }"#;

        let parsed: Settings = serde_json::from_str(legacy).expect("legacy settings");
        let migrated = parsed.migrate().normalized();
        assert!(migrated.auto_start_on_game);
    }

    #[test]
    fn an_explicit_false_survives_a_reload() {
        let mut settings = Settings::default();
        settings.auto_start_on_game = false;
        let json = serde_json::to_string(&settings).expect("serialize");
        let parsed: Settings = serde_json::from_str(&json).expect("deserialize");
        assert!(!parsed.migrate().normalized().auto_start_on_game);
        // The JSON name matches the frontend's `autoStartOnGame`.
        assert!(json.contains("\"autoStartOnGame\":false"));
    }
}
