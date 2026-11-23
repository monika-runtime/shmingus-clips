use std::sync::Arc;

use crate::capture::Recorder;
use crate::detect::Detector;
use crate::library::Library;
use crate::media_server::MediaServer;
use crate::settings::SettingsStore;

/// Everything the Tauri commands need, managed as one piece of app state.
pub struct AppState {
    pub settings: Arc<SettingsStore>,
    pub recorder: Arc<Recorder>,
    pub detector: Arc<Detector>,
    pub library: Arc<Library>,
    /// `http://127.0.0.1:<port>` for `<video>` playback, or empty on failure.
    pub media_base: String,
    /// Kept alive for the process lifetime; the server thread holds the socket.
    pub _media_server: MediaServer,
}
