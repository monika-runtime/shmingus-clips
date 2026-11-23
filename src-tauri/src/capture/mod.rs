//! Replay-buffer capture engine.
//!
//! Owned by the capture task. The public surface below is frozen: the command
//! layer and the tray call it, so signatures must not change.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::RwLock;
use tauri::AppHandle;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::error::{AppError, Result};
use crate::events;
use crate::model::{
    Clip, EncoderKind, GameInfo, RecorderState, RecorderStatus, SavedMonitor, SelfTestReport,
    Settings,
};
use crate::settings::SettingsStore;

pub mod audio;
pub mod clip;
pub mod pipeline;
pub mod portal;
pub mod ring;

/// One second per MPEG-TS fragment.
const SEGMENT_SECONDS: u64 = 1;

pub struct Recorder {
    /// Live settings; the engine re-reads this on every start/apply_settings.
    settings: Arc<SettingsStore>,
    status: Arc<RwLock<RecorderStatus>>,
    /// The running capture, if any. Async mutex: start/stop/save can be called
    /// concurrently from commands, tray clicks and hotkeys.
    session: Mutex<Option<Session>>,
}

/// Who started the current capture session. It decides whether game detection
/// may stop it: only an automatically started session is stopped when the game
/// exits, a manual one keeps running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    /// Start button, hotkey, tray item or the launch setting.
    Manual,
    /// A detected game.
    Auto,
}

struct Session {
    pipeline: pipeline::RunningPipeline,
    ring: ring::SegmentRing,
    /// Settings the pipeline was built from, used to detect what must restart.
    config: Settings,
    /// Keeps the portal session (and therefore the PipeWire stream) alive.
    portal: Option<portal::PortalStream>,
    ticker: Option<tauri::async_runtime::JoinHandle<()>>,
    ticker_flag: Arc<AtomicBool>,
    /// False once teardown started, so pipeline errors no longer flip the UI.
    alive: Arc<AtomicBool>,
    mic_enabled: bool,
    /// Preserved across internal restarts (settings change, monitor pick, mic
    /// toggle) so the auto/manual distinction survives a rebuilt pipeline.
    origin: Origin,
}

impl Session {
    async fn shutdown(mut self) {
        self.alive.store(false, Ordering::SeqCst);
        self.ticker_flag.store(false, Ordering::SeqCst);
        if let Some(ticker) = self.ticker.take() {
            ticker.abort();
        }
        self.pipeline.stop().await;
        // Nothing is writing to the segment directory any more.
        self.ring.clear();
        if let Some(portal) = self.portal.take() {
            portal.close().await;
        }
    }
}

impl Recorder {
    pub fn new(settings: Arc<SettingsStore>) -> Self {
        Self {
            settings,
            status: Arc::new(RwLock::new(RecorderStatus::default())),
            session: Mutex::new(None),
        }
    }

    pub fn status(&self) -> RecorderStatus {
        self.status.read().clone()
    }

    /// Replace the status and notify the frontend.
    pub(crate) fn publish(&self, app: &AppHandle, next: RecorderStatus) -> RecorderStatus {
        *self.status.write() = next.clone();
        crate::events::recorder_status(app, &next);
        next
    }

    /// Start the replay buffer because the user asked for it. The session is
    /// marked manual, so game detection will never stop it.
    pub async fn start(&self, app: &AppHandle) -> Result<RecorderStatus> {
        self.start_from(app, Origin::Manual).await
    }

    /// Start the replay buffer because a game was detected. No-op while a
    /// session is already starting, buffering or saving, so a manual session is
    /// never turned into an automatically stopped one.
    pub async fn start_auto(&self, app: &AppHandle) -> Result<RecorderStatus> {
        self.start_from(app, Origin::Auto).await
    }

    async fn start_from(&self, app: &AppHandle, origin: Origin) -> Result<RecorderStatus> {
        let mut guard = self.session.lock().await;

        let live = {
            let state = self.status.read().state;
            guard
                .as_ref()
                .map(|session| session_is_live(state, session.pipeline.is_finished()))
                .unwrap_or(false)
        };
        if live {
            if origin == Origin::Manual {
                // An explicit Start takes ownership of an auto-started session,
                // so a later game exit must not stop it.
                if let Some(session) = guard.as_mut() {
                    session.origin = Origin::Manual;
                }
            }
            return Ok(self.status.read().clone());
        }
        if guard.is_some() {
            log::info!("replacing a stopped capture session");
            if let Some(dead) = guard.take() {
                dead.shutdown().await;
            }
        }

        let settings = self.settings.get();
        self.publish(app, starting_status(&settings));

        match self.spawn_session(app, settings.clone(), origin).await {
            Ok(session) => {
                // spawn_session may have persisted the monitor it just picked.
                let settings = self.settings.get();
                let mic_active = session.mic_enabled;
                let status = buffering_status(&settings, session.pipeline.info(), mic_active);
                log::info!(
                    "{} capture started",
                    match origin {
                        Origin::Manual => "manual",
                        Origin::Auto => "automatic",
                    }
                );
                *guard = Some(session);
                Ok(self.publish(app, status))
            }
            Err(err) => {
                if origin == Origin::Auto && portal::is_cancelled(&err) {
                    // The user dismissed the picker an automatic start opened;
                    // leave the app idle instead of showing an error.
                    log::info!("automatic capture start was cancelled at the screen picker");
                    self.publish(app, RecorderStatus::default());
                    return Err(err);
                }
                let mut status = starting_status(&settings);
                status.state = RecorderState::Error;
                status.error = Some(err.to_string());
                status.buffered_seconds = 0;
                self.publish(app, status);
                events::toast(
                    app,
                    "error",
                    format!("Could not start the replay buffer: {err}"),
                );
                Err(err)
            }
        }
    }

    /// Stop the replay buffer because the user asked for it. Always stops and
    /// clears the origin, so nothing can be auto-stopped afterwards.
    pub async fn stop(&self, app: &AppHandle) -> Result<RecorderStatus> {
        self.stop_session(app).await
    }

    /// Stop the replay buffer because the detected game exited. Only a session
    /// that was started automatically is stopped; a manual one keeps running.
    pub async fn stop_auto(&self, app: &AppHandle) -> Result<RecorderStatus> {
        let mut guard = self.session.lock().await;
        let origin = guard.as_ref().map(|session| session.origin);
        if !auto_stop_allowed(origin) {
            log::debug!("game exited; the running capture session was started manually");
            return Ok(self.status.read().clone());
        }
        if let Some(session) = guard.take() {
            session.shutdown().await;
        }
        log::info!("game exited; stopped the auto-started replay buffer");
        Ok(self.publish(app, RecorderStatus::default()))
    }

    /// A detected game exited: save the buffered moments first (so a crash is
    /// not lost), then stop an automatically started session. Manual sessions
    /// are left alone.
    pub async fn save_and_stop_auto(&self, app: &AppHandle, game: Option<GameInfo>) -> Result<()> {
        let auto = {
            let guard = self.session.lock().await;
            auto_stop_allowed(guard.as_ref().map(|session| session.origin))
        };
        if !auto {
            log::debug!("game exited; the running capture session was started manually");
            return Ok(());
        }

        if let Err(err) = self.save_clip(app, game, None).await {
            log::warn!("could not save the replay buffer before stopping: {err}");
        }
        self.stop_auto(app).await.map(|_| ())
    }

    async fn stop_session(&self, app: &AppHandle) -> Result<RecorderStatus> {
        let mut guard = self.session.lock().await;
        if let Some(session) = guard.take() {
            session.shutdown().await;
        }
        Ok(self.publish(app, RecorderStatus::default()))
    }

    pub async fn save_clip(
        &self,
        app: &AppHandle,
        game: Option<GameInfo>,
        title: Option<String>,
    ) -> Result<Clip> {
        let mut guard = self.session.lock().await;
        let session = guard
            .as_mut()
            .ok_or_else(|| AppError::Capture("the replay buffer is not running".into()))?;

        if session.pipeline.is_finished() {
            return Err(AppError::Capture("the capture pipeline has stopped".into()));
        }
        let current = self.status.read().clone();
        if current.state == RecorderState::Error {
            return Err(AppError::Capture(
                current
                    .error
                    .clone()
                    .unwrap_or_else(|| "the capture pipeline failed".into()),
            ));
        }

        let settings = self.settings.get();

        // Confirm the request immediately: encoding and remuxing take a moment,
        // and the hotkey should feel instant. Only reached once a live session
        // has been validated, so a failed save does not chime.
        if settings.clip_sound {
            crate::sound::play_clip_saved();
        }

        self.publish(
            app,
            RecorderStatus {
                state: RecorderState::Saving,
                ..current
            },
        );

        let result = clip::save(
            app,
            &session.pipeline,
            &session.ring,
            &settings,
            game,
            title,
        )
        .await;

        let mut next = self.status.read().clone();
        let stopped = session.pipeline.is_finished();
        next.state = if stopped {
            RecorderState::Error
        } else {
            RecorderState::Buffering
        };
        next.buffer_target_seconds = settings.buffer_seconds;
        next.buffered_seconds = session.ring.buffered_seconds().min(settings.buffer_seconds);
        match &result {
            Ok(_) => next.error = None,
            Err(err) => {
                if stopped {
                    next.error = Some("the capture pipeline stopped".into());
                } else {
                    next.error = Some(err.to_string());
                }
                events::toast(app, "error", format!("Could not save the clip: {err}"));
            }
        }
        self.publish(app, next);

        result
    }

    /// Toggle the microphone track without restarting the buffer.
    pub async fn toggle_mic(&self, app: &AppHandle) -> Result<()> {
        let mut settings = self.settings.get();
        settings.audio.mic_enabled = !settings.audio.mic_enabled;
        let settings = self.settings.replace(settings)?;

        let mut guard = self.session.lock().await;
        let mut restart = false;

        if let Some(session) = guard.as_mut() {
            if settings.audio.mic_enabled && !session.pipeline.has_mic() {
                // The mic branch was left out when the buffer started; the only
                // way to add it is a fresh pipeline.
                restart = true;
            } else if session.pipeline.has_mic() {
                session
                    .pipeline
                    .set_mic_enabled(settings.audio.mic_enabled)?;
                session.mic_enabled = settings.audio.mic_enabled;
                session.config = settings.clone();
                let mut status = self.status.read().clone();
                status.mic_active = settings.audio.mic_enabled;
                self.publish(app, status);
            }
        } else {
            let mut status = self.status.read().clone();
            status.mic_active = settings.audio.mic_enabled;
            self.publish(app, status);
        }
        drop(guard);

        if restart {
            self.restart(app).await?;
        }
        Ok(())
    }

    /// Show the portal picker and remember the chosen monitor.
    pub async fn choose_monitor(&self, app: &AppHandle) -> Result<SavedMonitor> {
        let stream = portal::open(None).await?;
        let saved = SavedMonitor {
            id: stream.node_id,
            name: stream.label.clone(),
            restore_token: stream.restore_token.clone(),
        };
        stream.close().await;

        let mut settings = self.settings.get();
        settings.capture_monitor = Some(saved.clone());
        self.settings.replace(settings)?;

        if self.session.lock().await.is_some() {
            if let Err(err) = self.restart(app).await {
                log::warn!("could not restart capture on the new monitor: {err}");
                events::toast(
                    app,
                    "error",
                    format!("Monitor changed, but capture did not restart: {err}"),
                );
            }
        }

        Ok(saved)
    }

    /// Re-read settings; restart the buffer when something capture-relevant
    /// changed, otherwise just apply volumes/mic live.
    pub async fn apply_settings(&self, app: &AppHandle) -> Result<()> {
        let settings = self.settings.get();
        let mut guard = self.session.lock().await;
        let Some(session) = guard.as_mut() else {
            // Nothing is running, but the UI should still reflect the new
            // capture settings.
            let mut status = self.status.read().clone();
            status.mic_active = settings.audio.mic_enabled;
            status.desktop_audio_active = settings.audio.desktop_enabled;
            status.buffer_target_seconds = settings.buffer_seconds;
            self.publish(app, status);
            return Ok(());
        };

        let mic_changed = session.mic_enabled != settings.audio.mic_enabled;
        let needs_restart = capture_fields_changed(&session.config, &settings)
            || (mic_changed && settings.audio.mic_enabled && !session.pipeline.has_mic());

        if needs_restart {
            drop(guard);
            log::info!("capture settings changed; restarting the replay buffer");
            return self.restart(app).await;
        }

        session
            .pipeline
            .set_volumes(
                Some(settings.audio.desktop_volume),
                Some(settings.audio.mic_volume),
            )
            .map_err(|err| AppError::Capture(format!("could not apply the audio volume: {err}")))?;

        if mic_changed && session.pipeline.has_mic() {
            session
                .pipeline
                .set_mic_enabled(settings.audio.mic_enabled)?;
            session.mic_enabled = settings.audio.mic_enabled;
        }

        session.config = settings.clone();
        let mut status = self.status.read().clone();
        status.buffer_target_seconds = settings.buffer_seconds;
        status.mic_active = session.mic_enabled;
        status.desktop_audio_active = settings.audio.desktop_enabled;
        self.publish(app, status);
        Ok(())
    }

    /// Rebuild the pipeline while keeping its origin: an auto-started buffer
    /// that restarts for a settings change or a new monitor must still be
    /// stopped when the game exits, and a manual one must stay manual.
    async fn restart(&self, app: &AppHandle) -> Result<()> {
        let origin = {
            let mut guard = self.session.lock().await;
            match guard.take() {
                Some(session) => {
                    let origin = session.origin;
                    session.shutdown().await;
                    origin
                }
                None => Origin::Manual,
            }
        };
        self.start_from(app, origin).await?;
        Ok(())
    }

    async fn spawn_session(
        &self,
        app: &AppHandle,
        mut settings: Settings,
        origin: Origin,
    ) -> Result<Session> {
        // 1. Portal: restore the previous monitor, re-ask if the token expired.
        let token = settings
            .capture_monitor
            .as_ref()
            .and_then(|monitor| monitor.restore_token.clone());
        let mut portal_stream = match portal::open(token.as_deref()).await {
            Ok(stream) => stream,
            // A stale restore token is common (monitor unplugged, selection
            // revoked), so ask again - but never after the user cancelled,
            // which would pop the dialog a second time.
            Err(err) if token.is_some() && !portal::is_cancelled(&err) => {
                log::warn!("could not restore the previous monitor ({err}); showing the picker");
                portal::open(None).await?
            }
            Err(err) => return Err(err),
        };

        let saved = SavedMonitor {
            id: portal_stream.node_id,
            name: portal_stream.label.clone(),
            restore_token: portal_stream.restore_token.clone(),
        };
        if settings.capture_monitor.as_ref() != Some(&saved) {
            settings.capture_monitor = Some(saved);
            if let Err(err) = self.settings.replace(settings.clone()) {
                log::warn!("could not persist the monitor selection: {err}");
            }
        }

        // 2. Ring buffer directory, empty for this session.
        let segment_dir = crate::settings::cache_dir().join("segments");
        let ring = ring::SegmentRing::new(segment_dir.clone());
        if let Err(err) = ring.prepare() {
            portal_stream.close().await;
            return Err(err);
        }

        // 3. Pipeline.
        let fd = match portal_stream.take_fd() {
            Ok(fd) => fd,
            Err(err) => {
                portal_stream.close().await;
                return Err(err);
            }
        };

        let desktop_audio = settings
            .audio
            .desktop_enabled
            .then(|| pipeline::AudioBranch {
                source: pipeline::AudioSource::Pulse {
                    device: Some(audio::desktop_source()),
                },
                volume: settings.audio.desktop_volume,
            });
        let mic_audio = settings.audio.mic_enabled.then(|| pipeline::AudioBranch {
            source: pipeline::AudioSource::Pulse {
                device: settings.audio.mic_device.clone(),
            },
            volume: settings.audio.mic_volume,
        });

        let spec = pipeline::PipelineSpec {
            video: pipeline::VideoSource::PipeWire {
                node_id: portal_stream.node_id,
                fd,
                size: portal_stream.size,
            },
            desktop_audio,
            mic_audio,
            fps: settings.fps,
            bitrate_mbps: settings.bitrate_mbps,
            max_width: settings.max_width,
            encoder: settings.encoder,
            segment_dir: segment_dir.clone(),
            segment_seconds: SEGMENT_SECONDS,
        };

        let alive = Arc::new(AtomicBool::new(true));
        let on_error = {
            let status = self.status.clone();
            let app = app.clone();
            let alive = alive.clone();
            let callback: pipeline::ErrorCallback = Arc::new(move |message: String| {
                if !alive.load(Ordering::SeqCst) {
                    return;
                }
                let mut current = status.write();
                if matches!(current.state, RecorderState::Off) {
                    return;
                }
                current.state = RecorderState::Error;
                current.error = Some(message.clone());
                current.buffered_seconds = 0;
                let snapshot = current.clone();
                drop(current);
                events::recorder_status(&app, &snapshot);
                events::toast(&app, "error", format!("Capture stopped: {message}"));
            });
            callback
        };

        let started =
            tokio::task::spawn_blocking(move || pipeline::RunningPipeline::start(spec, on_error))
                .await
                .map_err(|err| AppError::Capture(format!("capture start task failed: {err}")))?;

        let started = match started {
            Ok(pipeline) => pipeline,
            Err(err) => {
                portal_stream.close().await;
                return Err(err);
            }
        };

        let mic_enabled = settings.audio.mic_enabled && started.has_mic();

        // 4. Keep the status' buffered-seconds and evict old fragments.
        let ticker_flag = Arc::new(AtomicBool::new(true));
        let ticker = tauri::async_runtime::spawn({
            let flag = ticker_flag.clone();
            let status = self.status.clone();
            let settings = self.settings.clone();
            let ring = ring.clone();
            let app = app.clone();
            async move {
                let mut interval = tokio::time::interval(Duration::from_secs(1));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    interval.tick().await;
                    if !flag.load(Ordering::SeqCst) {
                        break;
                    }
                    let buffer_seconds = settings.get().buffer_seconds;
                    ring.evict(buffer_seconds + 5);
                    let mut current = status.write();
                    if current.state != RecorderState::Buffering {
                        continue;
                    }
                    current.buffered_seconds = ring.buffered_seconds().min(buffer_seconds);
                    current.buffer_target_seconds = buffer_seconds;
                    let snapshot = current.clone();
                    drop(current);
                    events::recorder_status(&app, &snapshot);
                }
            }
        });

        Ok(Session {
            pipeline: started,
            ring,
            config: settings.clone(),
            portal: Some(portal_stream),
            ticker: Some(ticker),
            ticker_flag,
            alive,
            mic_enabled,
            origin,
        })
    }
}

fn starting_status(settings: &Settings) -> RecorderStatus {
    RecorderStatus {
        state: RecorderState::Starting,
        encoder: None,
        hardware_encoding: false,
        capture_label: settings
            .capture_monitor
            .as_ref()
            .map(|monitor| monitor.name.clone()),
        buffered_seconds: 0,
        buffer_target_seconds: settings.buffer_seconds,
        started_at: None,
        mic_active: settings.audio.mic_enabled,
        desktop_audio_active: settings.audio.desktop_enabled,
        error: None,
    }
}

fn buffering_status(
    settings: &Settings,
    info: &pipeline::PipelineInfo,
    mic_active: bool,
) -> RecorderStatus {
    RecorderStatus {
        state: RecorderState::Buffering,
        encoder: Some(info.encoder_label.clone()),
        hardware_encoding: info.hardware_encoding,
        capture_label: settings
            .capture_monitor
            .as_ref()
            .map(|monitor| monitor.name.clone()),
        buffered_seconds: 0,
        buffer_target_seconds: settings.buffer_seconds,
        started_at: Some(chrono::Utc::now().to_rfc3339()),
        mic_active,
        desktop_audio_active: settings.audio.desktop_enabled,
        error: None,
    }
}

/// Fields that force a pipeline rebuild. Volumes and the mic mute state are
/// applied live instead.
fn capture_fields_changed(previous: &Settings, next: &Settings) -> bool {
    previous.fps != next.fps
        || previous.bitrate_mbps != next.bitrate_mbps
        || previous.encoder != next.encoder
        || previous.max_width != next.max_width
        || previous.audio.desktop_enabled != next.audio.desktop_enabled
        || previous.audio.mic_device != next.audio.mic_device
        || previous.capture_monitor.as_ref().map(|monitor| monitor.id)
            != next.capture_monitor.as_ref().map(|monitor| monitor.id)
}

/// A session still owns the capture while its state is live and the pipeline
/// thread is running. A second start must not replace it in that window.
fn session_is_live(state: RecorderState, pipeline_finished: bool) -> bool {
    matches!(
        state,
        RecorderState::Starting | RecorderState::Buffering | RecorderState::Saving
    ) && !pipeline_finished
}

/// A game exit may only stop a session it started itself. A missing session or
/// a manually started one is left alone.
fn auto_stop_allowed(origin: Option<Origin>) -> bool {
    matches!(origin, Some(Origin::Auto))
}

// ---------------------------------------------------------------------------
// Headless engine test (`clipforge --selftest`)
// ---------------------------------------------------------------------------

/// Headless engine test used by `clipforge --selftest`. Needs no portal: a test
/// pattern and two tones go through the same encoder, splitmuxsink, concat and
/// remux path as a real recording.
pub async fn selftest() -> Result<SelfTestReport> {
    // Logging is configured by the app; in `--selftest` mode this is the first
    // chance to enable it so RUST_LOG works for diagnosing a failure.
    let _ = env_logger::try_init();

    let mut report = SelfTestReport::new();
    let selftest_root = crate::settings::cache_dir().join("selftest");
    // A previous run's clip is no longer useful; keep the cache from growing.
    let _ = std::fs::remove_dir_all(&selftest_root);
    let root = selftest_root.join(format!(
        "{}-{}",
        chrono::Local::now().format("%Y%m%d-%H%M%S"),
        &Uuid::new_v4().to_string()[..8]
    ));
    let segments_dir = root.join("segments");
    let work_dir = root.join("work");
    std::fs::create_dir_all(&segments_dir)?;
    std::fs::create_dir_all(&work_dir)?;

    let outcome = run_selftest(&root, &segments_dir, &work_dir, &mut report).await;
    if let Err(err) = outcome {
        if report.error.is_none() {
            report.error = Some(err.to_string());
        }
        report.step("selftest", false, err.to_string());
    }

    // Keep the finished clip and thumbnail as evidence; the intermediates go.
    let _ = std::fs::remove_dir_all(&segments_dir);
    let _ = std::fs::remove_dir_all(&work_dir);

    Ok(report)
}

async fn run_selftest(
    root: &Path,
    segments_dir: &Path,
    work_dir: &Path,
    report: &mut SelfTestReport,
) -> Result<()> {
    let gst_ready = gstreamer::init().is_ok();
    report.step(
        "gstreamer",
        gst_ready,
        "GStreamer runtime initialised".to_string(),
    );
    if !gst_ready {
        return Err(AppError::Capture("could not initialise GStreamer".into()));
    }

    let ring = ring::SegmentRing::new(segments_dir);
    ring.prepare()?;

    let error_slot: Arc<parking_lot::Mutex<Option<String>>> =
        Arc::new(parking_lot::Mutex::new(None));
    let on_error = {
        let slot = error_slot.clone();
        let callback: pipeline::ErrorCallback =
            Arc::new(move |message: String| *slot.lock() = Some(message));
        callback
    };

    let spec = test_spec(segments_dir);

    let started =
        tokio::task::spawn_blocking(move || pipeline::RunningPipeline::start(spec, on_error))
            .await
            .map_err(|err| AppError::Capture(format!("selftest start task failed: {err}")))?;

    let pipeline = match started {
        Ok(pipeline) => pipeline,
        Err(err) => {
            report.step("pipeline-start", false, err.to_string());
            return Err(err);
        }
    };
    report.step(
        "pipeline-start",
        true,
        format!(
            "{} encoding a 1280x720 test pattern with two tone tracks",
            pipeline.info().encoder_label
        ),
    );

    let outcome = async {
        // Let the buffer build up: one fragment per second, and the newest one
        // is still being written.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(18);
        while ring.buffered_seconds() < 6 {
            if pipeline.is_finished() {
                let message = error_slot
                    .lock()
                    .clone()
                    .unwrap_or_else(|| "the capture pipeline stopped during the test".into());
                return Err(AppError::Capture(message));
            }
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        let segments = ring.count();
        report.step(
            "segments",
            segments >= 6,
            format!("{segments} one-second MPEG-TS fragments written"),
        );
        if segments < 6 {
            return Err(AppError::Capture(format!(
                "only {segments} segments were written before the timeout"
            )));
        }

        let last_index = ring.latest_index().unwrap_or(0);
        pipeline.split_now()?;
        let split =
            clip::wait_for_new_segment(&ring, last_index, Duration::from_millis(2500)).await;
        report.step(
            "split-now",
            split,
            if split {
                "splitmuxsink closed the live fragment".to_string()
            } else {
                "splitmuxsink did not open a new fragment within 2.5s".to_string()
            },
        );
        tokio::time::sleep(Duration::from_millis(300)).await;

        let parts = ring.tail_up_to(6, last_index);
        report.step(
            "ring-tail",
            parts.len() >= 5,
            format!("{} fragments selected for a ~6s clip", parts.len()),
        );
        if parts.is_empty() {
            return Err(AppError::Capture("the ring buffer was empty".into()));
        }

        let out = root.join("clip.mp4");
        let thumbnail = root.join("thumbnail.jpg");
        let probe = {
            let work_dir = work_dir.to_path_buf();
            let out = out.clone();
            let thumbnail = thumbnail.clone();
            tokio::task::spawn_blocking(move || {
                clip::assemble(&parts, &work_dir, &out, &thumbnail, 6000)
            })
            .await
            .map_err(|err| AppError::Capture(format!("selftest render task failed: {err}")))??
        };

        let file_ok = out.exists() && probe.size_bytes > 0;
        report.step(
            "mp4",
            file_ok,
            format!(
                "{} ({} bytes, {}x{}, {} ms)",
                out.display(),
                probe.size_bytes,
                probe.width,
                probe.height,
                probe.duration_ms
            ),
        );
        let duration_ok = (5_000..=7_000).contains(&probe.duration_ms);
        report.step(
            "duration",
            duration_ok,
            format!("{} ms (expected 5000-7000 ms)", probe.duration_ms),
        );
        let audio_ok = probe.audio_streams == 2;
        report.step(
            "audio-tracks",
            audio_ok,
            format!(
                "{} audio streams (expected 2 separate tracks)",
                probe.audio_streams
            ),
        );
        let thumbnail_ok = std::fs::metadata(&thumbnail)
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false);
        report.step("thumbnail", thumbnail_ok, thumbnail.display().to_string());

        if file_ok {
            report.artifacts.push(out.to_string_lossy().to_string());
        }
        if thumbnail_ok {
            report
                .artifacts
                .push(thumbnail.to_string_lossy().to_string());
        }

        Ok(())
    }
    .await;

    pipeline.stop().await;

    let cycles = if outcome.is_ok() {
        run_start_stop_cycles(segments_dir, report).await
    } else {
        Ok(())
    };
    outcome.and(cycles)
}

fn test_spec(segments_dir: &Path) -> pipeline::PipelineSpec {
    pipeline::PipelineSpec {
        video: pipeline::VideoSource::Test {
            width: 1280,
            height: 720,
        },
        desktop_audio: Some(pipeline::AudioBranch {
            source: pipeline::AudioSource::TestTone { freq: 440.0 },
            volume: 0.8,
        }),
        mic_audio: Some(pipeline::AudioBranch {
            source: pipeline::AudioSource::TestTone { freq: 880.0 },
            volume: 0.8,
        }),
        fps: 30,
        bitrate_mbps: 8,
        max_width: 1920,
        encoder: EncoderKind::Auto,
        segment_dir: segments_dir.to_path_buf(),
        segment_seconds: SEGMENT_SECONDS,
    }
}

/// Start and stop the test pipeline three more times. Every stop must join its
/// thread and close its files, and the segment directory must be empty again.
async fn run_start_stop_cycles(segments_dir: &Path, report: &mut SelfTestReport) -> Result<()> {
    let ring = ring::SegmentRing::new(segments_dir);
    let mut failures = Vec::new();

    for cycle in 1..=3 {
        ring.prepare()?;
        let spec = test_spec(segments_dir);
        let on_error: pipeline::ErrorCallback = Arc::new(|_| {});
        let started =
            tokio::task::spawn_blocking(move || pipeline::RunningPipeline::start(spec, on_error))
                .await
                .map_err(|err| AppError::Capture(format!("cycle start task failed: {err}")))?;

        let pipeline = match started {
            Ok(pipeline) => pipeline,
            Err(err) => {
                failures.push(format!("cycle {cycle}: {err}"));
                continue;
            }
        };

        let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
        while ring.count() == 0 {
            if pipeline.is_finished() || tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let fragments = ring.count();
        pipeline.stop().await;
        ring.clear();
        // Give a thread that failed to stop a chance to write again.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let remaining = ring.count();
        if fragments == 0 {
            failures.push(format!("cycle {cycle}: no fragment was written"));
        }
        if remaining != 0 {
            failures.push(format!(
                "cycle {cycle}: {remaining} fragments appeared after stop"
            ));
        }
        ring.clear();
    }

    let ok = failures.is_empty();
    report.step(
        "start-stop-cycles",
        ok,
        if ok {
            "3 start/stop cycles: pipeline threads joined, no fragments left behind".to_string()
        } else {
            failures.join("; ")
        },
    );
    if ok {
        Ok(())
    } else {
        Err(AppError::Capture(failures.join("; ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_running_session_blocks_a_second_start() {
        for state in [
            RecorderState::Starting,
            RecorderState::Buffering,
            RecorderState::Saving,
        ] {
            assert!(session_is_live(state, false), "{state:?} should be live");
            // A finished pipeline is dead even if the status was not updated yet.
            assert!(!session_is_live(state, true));
        }
        for state in [RecorderState::Off, RecorderState::Error] {
            assert!(!session_is_live(state, false), "{state:?} is not live");
        }
    }

    #[test]
    fn only_an_auto_started_session_is_stopped_by_a_game_exit() {
        assert!(auto_stop_allowed(Some(Origin::Auto)));
        assert!(!auto_stop_allowed(Some(Origin::Manual)));
        assert!(!auto_stop_allowed(None));
    }

    #[test]
    fn settings_that_do_not_affect_capture_do_not_restart_it() {
        let base = Settings::default();

        let mut toggled = base.clone();
        toggled.auto_start_on_game = !base.auto_start_on_game;
        toggled.start_buffer_on_launch = !base.start_buffer_on_launch;
        assert!(!capture_fields_changed(&base, &toggled));

        let mut resized = base.clone();
        resized.max_width = base.max_width / 2;
        assert!(capture_fields_changed(&base, &resized));
    }
}
