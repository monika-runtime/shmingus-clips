//! Types shared between the Rust backend and the React frontend.
//!
//! Everything here is serialized as camelCase JSON. The TypeScript mirror lives
//! in `src/lib/types.ts` - keep the two in sync.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum EncoderKind {
    #[default]
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "nvenc")]
    Nvenc,
    #[serde(rename = "x264")]
    X264,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Hotkeys {
    /// Accelerator in XDG portal syntax, e.g. `CTRL+ALT+c`.
    pub save_clip: String,
    pub toggle_buffer: String,
    pub toggle_mic: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            save_clip: "ALT+c".into(),
            toggle_buffer: "CTRL+ALT+b".into(),
            toggle_mic: "CTRL+ALT+m".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioSettings {
    pub desktop_enabled: bool,
    /// 0.0 ..= 2.0 (2.0 = +6 dB headroom)
    pub desktop_volume: f32,
    pub mic_enabled: bool,
    /// PulseAudio/PipeWire source id or name, `None` = default source.
    pub mic_device: Option<String>,
    pub mic_volume: f32,
    /// Experimental: lift a quiet microphone toward the desktop level in the
    /// mixed track at save time. The isolated tracks stay untouched.
    pub mic_auto_gain: bool,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            desktop_enabled: true,
            desktop_volume: 1.0,
            mic_enabled: true,
            mic_device: None,
            mic_volume: 1.0,
            mic_auto_gain: false,
        }
    }
}

/// A monitor the user authorised through the ScreenCast portal. The restore
/// token lets later sessions pick the same monitor without showing the dialog
/// again.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SavedMonitor {
    pub id: u32,
    pub name: String,
    pub restore_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Absolute directory where finished clips are written.
    pub save_dir: String,
    /// Replay buffer length in seconds (5..=300).
    pub buffer_seconds: u32,
    pub fps: u32,
    pub bitrate_mbps: u32,
    pub encoder: EncoderKind,
    pub audio: AudioSettings,
    pub hotkeys: Hotkeys,
    pub capture_monitor: Option<SavedMonitor>,
    /// Watch running games and tag clips with the game name.
    pub auto_detect_games: bool,
    /// Start the replay buffer automatically when a game is detected, and stop
    /// it (if it was started that way) when the game exits.
    pub auto_start_on_game: bool,
    /// Save the last buffered seconds when a detected game exits, before the
    /// automatic buffer is stopped.
    pub save_on_game_exit: bool,
    pub start_buffer_on_launch: bool,
    pub minimize_to_tray: bool,
    pub launch_at_login: bool,
    pub notify_on_clip: bool,
    /// Play a short chime when a clip is saved (notifications are often hidden
    /// by fullscreen games).
    pub clip_sound: bool,
    /// Base URL of the self-hosted share service, e.g. https://clips.shmingus.net
    pub share_base_url: String,
    /// Optional upload token (the service can also run without auth).
    pub share_token: Option<String>,
    /// Downscale captures wider than this to keep files small. `0` = native.
    pub max_width: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            save_dir: default_save_dir(),
            buffer_seconds: 30,
            fps: 60,
            bitrate_mbps: 20,
            encoder: EncoderKind::Auto,
            audio: AudioSettings::default(),
            hotkeys: Hotkeys::default(),
            capture_monitor: None,
            auto_detect_games: true,
            auto_start_on_game: true,
            save_on_game_exit: true,
            start_buffer_on_launch: false,
            minimize_to_tray: true,
            launch_at_login: false,
            notify_on_clip: true,
            clip_sound: true,
            share_base_url: "https://clips.shmingus.net".into(),
            share_token: None,
            max_width: 1920,
        }
    }
}

pub fn default_save_dir() -> String {
    let base = dirs::video_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    base.join("Clipforge").to_string_lossy().to_string()
}

// ---------------------------------------------------------------------------
// Capture
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RecorderState {
    #[default]
    #[serde(rename = "off")]
    Off,
    #[serde(rename = "starting")]
    Starting,
    #[serde(rename = "buffering")]
    Buffering,
    #[serde(rename = "saving")]
    Saving,
    #[serde(rename = "error")]
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RecorderStatus {
    pub state: RecorderState,
    /// Human readable encoder actually in use, e.g. "NVENC H.264".
    pub encoder: Option<String>,
    pub hardware_encoding: bool,
    pub capture_label: Option<String>,
    /// Seconds currently held in the replay buffer.
    pub buffered_seconds: u32,
    pub buffer_target_seconds: u32,
    pub started_at: Option<String>,
    pub mic_active: bool,
    pub desktop_audio_active: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDevice {
    pub id: String,
    pub label: String,
    /// True for monitor (loopback) sources - these carry desktop audio.
    pub is_monitor: bool,
    pub is_default: bool,
}

/// A short measurement of the microphone input, for the level meter.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicLevel {
    /// Mean level over the sample window, in dBFS (negative; -inf clamps low).
    pub rms_db: f32,
    /// Highest sample in the window, in dBFS.
    pub peak_db: f32,
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameSource {
    #[serde(rename = "steam")]
    Steam,
    #[serde(rename = "lutris")]
    Lutris,
    #[serde(rename = "heroic")]
    Heroic,
    #[serde(rename = "process")]
    Process,
    #[serde(rename = "manual")]
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GameInfo {
    pub id: String,
    pub name: String,
    pub source: GameSource,
    pub pid: Option<u32>,
}

// ---------------------------------------------------------------------------
// Library
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub id: String,
    pub title: String,
    pub game: Option<String>,
    /// Absolute path to the MP4.
    pub path: String,
    /// Absolute path to a JPEG thumbnail, if one has been generated.
    pub thumbnail: Option<String>,
    /// RFC 3339 timestamp.
    pub created_at: String,
    pub duration_ms: u64,
    pub size_bytes: u64,
    pub width: u32,
    pub height: u32,
    pub favorite: bool,
}

// ---------------------------------------------------------------------------
// Environment / diagnostics
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckResult {
    pub id: String,
    pub label: String,
    pub ok: bool,
    /// A required check that fails makes the whole environment unusable.
    pub required: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentReport {
    pub ok: bool,
    pub checks: Vec<CheckResult>,
    pub encoders: Vec<String>,
    pub session_type: String,
    pub desktop: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelfTestStep {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelfTestReport {
    pub ok: bool,
    pub steps: Vec<SelfTestStep>,
    pub artifacts: Vec<String>,
    pub error: Option<String>,
}

impl SelfTestReport {
    pub fn new() -> Self {
        Self {
            ok: true,
            steps: Vec::new(),
            artifacts: Vec::new(),
            error: None,
        }
    }

    pub fn step(&mut self, name: impl Into<String>, ok: bool, detail: impl Into<String>) {
        if !ok {
            self.ok = false;
        }
        self.steps.push(SelfTestStep {
            name: name.into(),
            ok,
            detail: detail.into(),
        });
    }
}

impl Default for SelfTestReport {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// A clip that lives on the share service.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareInfo {
    pub id: String,
    /// Public viewer page, used for Discord embeds.
    pub url: String,
    pub embed_url: String,
    pub video_url: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthStatus {
    pub signed_in: bool,
    pub name: Option<String>,
    pub email: Option<String>,
    /// Authentik `sub` (the username).
    pub sub: Option<String>,
    /// RFC 3339 access-token expiry, when signed in.
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareServerStatus {
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Toast {
    /// "info" | "success" | "error"
    pub level: String,
    pub message: String,
}
