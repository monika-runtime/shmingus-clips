use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::RwLock;

use crate::error::Result;
use crate::model::Settings;

/// JSON-backed settings store at `~/.config/clipforge/settings.json`.
pub struct SettingsStore {
    path: PathBuf,
    inner: RwLock<Settings>,
}

impl SettingsStore {
    pub fn load() -> Arc<Self> {
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

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("clipforge")
}

pub fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| std::env::temp_dir())
        .join("clipforge")
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
    fn auto_start_on_game_defaults_to_true() {
        assert!(Settings::default().auto_start_on_game);
        assert!(Settings::default().normalized().auto_start_on_game);
    }

    #[test]
    fn an_older_settings_file_keeps_auto_start_on_game_on() {
        // Written before `autoStartOnGame` existed: the container-level
        // `serde(default)` fills it from `Settings::default()`.
        let legacy = r#"{
            "saveDir": "/tmp/clipforge-clips",
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
