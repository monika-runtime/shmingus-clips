//! GStreamer pipeline construction: portal video + desktop audio + mic, encoded
//! with NVENC when available and x264 otherwise, muxed into short MPEG-TS
//! segments for the ring buffer.
//! Owned by the capture task.
//!
//! The pipeline runs on its own thread. That thread polls the GStreamer bus for
//! errors/EOS and drains a command channel (split-now, mute the mic, change
//! volumes, stop), so no other thread ever touches GStreamer objects.

use std::os::fd::{AsRawFd, OwnedFd};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use gstreamer as gst;
use gstreamer::prelude::*;

use crate::error::{AppError, Result};
use crate::model::EncoderKind;

use super::ring::SEGMENT_PATTERN;

/// Called once when the pipeline stops because of a fatal error. Runs on the
/// capture thread, so it must not block.
pub type ErrorCallback = Arc<dyn Fn(String) + Send + Sync + 'static>;

#[derive(Debug, Clone)]
pub enum AudioSource {
    /// A PulseAudio source name; `None` means the server default.
    Pulse { device: Option<String> },
    /// Headless selftest tone.
    TestTone { freq: f64 },
}

#[derive(Debug, Clone)]
pub struct AudioBranch {
    pub source: AudioSource,
    /// Linear gain, 0.0 ..= 2.0.
    pub volume: f32,
}

pub enum VideoSource {
    PipeWire {
        node_id: u32,
        /// Must stay open until the pipeline is torn down.
        fd: OwnedFd,
        /// Size reported by the portal, used to cap the output width.
        size: Option<(i32, i32)>,
    },
    /// Headless selftest pattern.
    Test { width: i32, height: i32 },
}

pub struct PipelineSpec {
    pub video: VideoSource,
    pub desktop_audio: Option<AudioBranch>,
    pub mic_audio: Option<AudioBranch>,
    pub fps: u32,
    pub bitrate_mbps: u32,
    /// Downscale anything wider than this. 0 = native.
    pub max_width: u32,
    pub encoder: EncoderKind,
    pub segment_dir: PathBuf,
    pub segment_seconds: u64,
}

#[derive(Debug, Clone)]
pub struct PipelineInfo {
    /// "NVENC H.264 (hardware)" / "x264 (software)".
    pub encoder_label: String,
    pub hardware_encoding: bool,
}

#[derive(Debug, Clone, Copy)]
enum Command {
    Stop,
    SplitNow,
    SetMic(bool),
    SetVolumes {
        desktop: Option<f32>,
        mic: Option<f32>,
    },
}

/// A pipeline running on its own thread.
pub struct RunningPipeline {
    commands: Sender<Command>,
    info: PipelineInfo,
    join: Option<JoinHandle<()>>,
    finished: Arc<AtomicBool>,
    has_mic: bool,
}

impl RunningPipeline {
    /// Build and start the pipeline. Blocks until it reaches PLAYING or fails.
    /// Call from a blocking context.
    pub fn start(spec: PipelineSpec, on_error: ErrorCallback) -> Result<Self> {
        let (command_tx, command_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel::<std::result::Result<PipelineInfo, String>>();
        let finished = Arc::new(AtomicBool::new(false));
        let has_mic = spec.mic_audio.is_some();
        let finished_thread = finished.clone();

        let join = std::thread::Builder::new()
            .name("clipforge-gst".into())
            .spawn(move || pipeline_thread(spec, command_rx, ready_tx, finished_thread, on_error))
            .map_err(|err| {
                AppError::Capture(format!("could not start the capture thread: {err}"))
            })?;

        match ready_rx.recv_timeout(Duration::from_secs(25)) {
            Ok(Ok(info)) => Ok(Self {
                commands: command_tx,
                info,
                join: Some(join),
                finished,
                has_mic,
            }),
            Ok(Err(message)) => {
                let _ = join.join();
                Err(AppError::Capture(message))
            }
            Err(err) => {
                let _ = command_tx.send(Command::Stop);
                let _ = join.join();
                Err(AppError::Capture(format!(
                    "the capture pipeline did not start: {err}"
                )))
            }
        }
    }

    pub fn info(&self) -> &PipelineInfo {
        &self.info
    }

    pub fn has_mic(&self) -> bool {
        self.has_mic
    }

    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::SeqCst)
    }

    /// Ask splitmuxsink to close the fragment it is writing and open a new one,
    /// so the last fragment becomes readable right away.
    pub fn split_now(&self) -> Result<()> {
        self.send(Command::SplitNow)
    }

    /// Mute/unmute the microphone track without restarting the pipeline.
    pub fn set_mic_enabled(&self, enabled: bool) -> Result<()> {
        if !self.has_mic {
            return Err(AppError::Capture(
                "this capture has no microphone track".into(),
            ));
        }
        self.send(Command::SetMic(enabled))
    }

    pub fn set_volumes(&self, desktop: Option<f32>, mic: Option<f32>) -> Result<()> {
        self.send(Command::SetVolumes { desktop, mic })
    }

    fn send(&self, command: Command) -> Result<()> {
        self.commands
            .send(command)
            .map_err(|_| AppError::Capture("the capture pipeline is no longer running".into()))
    }

    /// Stop the pipeline, its thread and every GStreamer object. Joins the
    /// thread, so all file handles have been released on return.
    pub async fn stop(mut self) {
        let _ = self.commands.send(Command::Stop);
        if let Some(join) = self.join.take() {
            let _ = tokio::task::spawn_blocking(move || join.join()).await;
        }
    }
}

impl Drop for RunningPipeline {
    fn drop(&mut self) {
        // Best-effort: a dropped pipeline must not leave its thread running.
        let _ = self.commands.send(Command::Stop);
    }
}

// ---------------------------------------------------------------------------
// Capture thread
// ---------------------------------------------------------------------------

fn pipeline_thread(
    spec: PipelineSpec,
    commands: Receiver<Command>,
    ready: Sender<std::result::Result<PipelineInfo, String>>,
    finished: Arc<AtomicBool>,
    on_error: ErrorCallback,
) {
    let built = match build(&spec) {
        Ok(built) => built,
        Err(err) => {
            let _ = ready.send(Err(err.to_string()));
            finished.store(true, Ordering::SeqCst);
            return;
        }
    };

    let Built {
        pipeline,
        sink,
        desktop_volume,
        mic_volume,
        info,
    } = built;

    log::debug!("capture pipeline built; starting");
    let starting = pipeline.set_state(gst::State::Playing);
    let (state_result, current, _pending) = pipeline.state(gst::ClockTime::from_seconds(15));
    log::info!(
        "capture pipeline state {current:?} (set_state ok={}, state query ok={})",
        starting.is_ok(),
        state_result.is_ok()
    );
    let playing = starting.is_ok() && state_result.is_ok() && current == gst::State::Playing;
    if !playing {
        let message = format!(
            "the capture pipeline could not start (state {current:?}): {}",
            state_result
                .err()
                .map(|err| err.to_string())
                .unwrap_or_else(|| starting.err().map(|e| e.to_string()).unwrap_or_default())
        );
        let _ = pipeline.set_state(gst::State::Null);
        let _ = pipeline.state(gst::ClockTime::from_seconds(5));
        let _ = ready.send(Err(message));
        finished.store(true, Ordering::SeqCst);
        return;
    }

    if ready.send(Ok(info)).is_err() {
        // The caller vanished while we were starting.
        let bus = pipeline.bus();
        drain_and_stop(&pipeline, bus.as_ref());
        finished.store(true, Ordering::SeqCst);
        return;
    }

    let mut mic_gain = spec
        .mic_audio
        .as_ref()
        .map(|branch| branch.volume as f64)
        .unwrap_or(0.0);
    let mut mic_enabled = spec.mic_audio.is_some();
    let bus = pipeline.bus();
    let mut running = true;

    while running {
        if let Some(bus) = bus.as_ref() {
            if let Some(message) = bus.timed_pop(gst::ClockTime::from_mseconds(100)) {
                match message.view() {
                    gst::MessageView::Error(err) => {
                        let detail = format!(
                            "{} ({})",
                            err.error(),
                            err.debug().unwrap_or_else(|| "no details".into())
                        );
                        log::error!("capture pipeline error: {detail}");
                        on_error(detail);
                        running = false;
                    }
                    gst::MessageView::Eos(_) => {
                        log::info!("capture pipeline reached EOS");
                        running = false;
                    }
                    gst::MessageView::Warning(warning) => {
                        log::warn!(
                            "capture pipeline warning from {:?}: {} ({})",
                            message.src().map(|src| src.path_string().to_string()),
                            warning.error(),
                            warning.debug().unwrap_or_else(|| "no details".into())
                        );
                    }
                    _ => {}
                }
            }
        }

        loop {
            match commands.try_recv() {
                Ok(Command::Stop) => {
                    running = false;
                    break;
                }
                Ok(Command::SplitNow) => {
                    let _ = sink.emit_by_name::<()>("split-now", &[]);
                }
                Ok(Command::SetMic(enabled)) => {
                    mic_enabled = enabled;
                    if let Some(volume) = mic_volume.as_ref() {
                        let gain = if enabled { mic_gain } else { 0.0 };
                        volume.set_property("volume", gain);
                    }
                }
                Ok(Command::SetVolumes { desktop, mic }) => {
                    if let (Some(volume), Some(gain)) = (desktop_volume.as_ref(), desktop) {
                        volume.set_property("volume", gain as f64);
                    }
                    if let Some(gain) = mic {
                        // Remember the new level even while muted, so unmuting
                        // uses it.
                        mic_gain = gain as f64;
                        if mic_enabled {
                            if let Some(volume) = mic_volume.as_ref() {
                                volume.set_property("volume", mic_gain);
                            }
                        }
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    running = false;
                    break;
                }
            }
        }
    }

    log::info!("stopping the capture pipeline");
    drain_and_stop(&pipeline, bus.as_ref());
    finished.store(true, Ordering::SeqCst);
    log::debug!("capture pipeline stopped");
    drop(spec); // keeps the PipeWire fd open until the elements are gone
}

/// EOS first: it lets splitmuxsink finalise the fragment and every streaming
/// thread drain. Going straight to NULL on a live pipeline can block forever.
fn drain_and_stop(pipeline: &gst::Pipeline, bus: Option<&gst::Bus>) {
    let _ = pipeline.send_event(gst::event::Eos::new());
    if let Some(bus) = bus {
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            let Some(message) = bus.timed_pop(gst::ClockTime::from_mseconds(100)) else {
                continue;
            };
            match message.view() {
                gst::MessageView::Eos(_) | gst::MessageView::Error(_) => break,
                _ => {}
            }
        }
    }
    log::debug!("capture pipeline sending NULL");
    let _ = pipeline.set_state(gst::State::Null);
    let _ = pipeline.state(gst::ClockTime::from_seconds(5));
}

// ---------------------------------------------------------------------------
// Pipeline construction
// ---------------------------------------------------------------------------

struct Built {
    pipeline: gst::Pipeline,
    sink: gst::Element,
    desktop_volume: Option<gst::Element>,
    mic_volume: Option<gst::Element>,
    info: PipelineInfo,
}

fn build(spec: &PipelineSpec) -> Result<Built> {
    gst::init()
        .map_err(|err| AppError::Capture(format!("could not initialise GStreamer: {err}")))?;

    let encoder = choose_encoder(spec.encoder)?;
    let pipeline = gst::Pipeline::new();

    // --- video -----------------------------------------------------------
    let source = match &spec.video {
        VideoSource::PipeWire { node_id, fd, .. } => {
            let source = element("pipewiresrc")?;
            source.set_property("path", node_id.to_string());
            source.set_property("fd", fd.as_raw_fd());
            source.set_property("client-name", "Shmingus Clips");
            source.set_property("autoconnect", true);
            source
        }
        VideoSource::Test { .. } => {
            let source = element("videotestsrc")?;
            source.set_property("is-live", true);
            source.set_property_from_str("pattern", "ball");
            source
        }
    };

    let videorate = element("videorate")?;
    let rate_caps = capsfilter(
        gst::Caps::builder("video/x-raw")
            .field("framerate", gst::Fraction::new(spec.fps as i32, 1))
            .build(),
    )?;
    let videoconvert = element("videoconvert")?;

    let source_size = match &spec.video {
        VideoSource::PipeWire { size, .. } => *size,
        VideoSource::Test { width, height } => Some((*width, *height)),
    };
    let target = scale_target(source_size, spec.max_width);

    let mut video_chain = vec![source, videorate, rate_caps, videoconvert];
    if let Some((width, height)) = target {
        video_chain.push(element("videoscale")?);
        video_chain.push(capsfilter(
            gst::Caps::builder("video/x-raw")
                .field("format", "NV12")
                .field("width", width)
                .field("height", height)
                .build(),
        )?);
    } else {
        video_chain.push(capsfilter(
            gst::Caps::builder("video/x-raw")
                .field("format", "NV12")
                .build(),
        )?);
    }

    let video_encoder = element(encoder.factory)?;
    if encoder.hardware {
        video_encoder.set_property("bitrate", spec.bitrate_mbps * 1000);
        video_encoder.set_property("gop-size", spec.fps as i32);
        video_encoder.set_property("zerolatency", true);
    } else {
        video_encoder.set_property("bitrate", spec.bitrate_mbps * 1000);
        video_encoder.set_property("key-int-max", spec.fps);
        video_encoder.set_property("bframes", 0u32);
        video_encoder.set_property_from_str("tune", "zerolatency");
        video_encoder.set_property_from_str("speed-preset", "veryfast");
    }
    video_chain.push(video_encoder);

    let h264parse = element("h264parse")?;
    h264parse.set_property("config-interval", -1i32);
    video_chain.push(h264parse);

    let video_queue = element("queue")?;
    video_chain.push(video_queue.clone());

    // --- muxer / segmenter ----------------------------------------------
    let sink = element("splitmuxsink")?;
    // splitmuxsink expands the location with printf-style formatting, so any
    // literal `%` in the directory has to be escaped - but the `%05u` in our
    // file name pattern must stay a format directive.
    let directory = spec.segment_dir.to_string_lossy().replace('%', "%%");
    let location = format!("{directory}/{SEGMENT_PATTERN}");
    sink.set_property("location", location);
    sink.set_property(
        "max-size-time",
        spec.segment_seconds.max(1) * 1_000_000_000u64,
    );
    sink.set_property("start-index", 0i32);
    let muxer = element("mpegtsmux")?;
    sink.set_property("muxer", &muxer);

    pipeline
        .add_many(std::iter::once(&sink))
        .map_err(|err| AppError::Capture(format!("could not build the pipeline: {err}")))?;
    pipeline
        .add_many(video_chain.iter())
        .map_err(|err| AppError::Capture(format!("could not build the pipeline: {err}")))?;
    gst::Element::link_many(video_chain.iter())
        .map_err(|err| AppError::Capture(format!("could not link the video chain: {err}")))?;
    let video_pad = sink
        .request_pad_simple("video")
        .ok_or_else(|| AppError::Capture("splitmuxsink has no video pad".into()))?;
    video_queue
        .static_pad("src")
        .ok_or_else(|| AppError::Capture("video queue has no source pad".into()))?
        .link(&video_pad)
        .map_err(|err| AppError::Capture(format!("could not link video: {err:?}")))?;

    // --- audio -----------------------------------------------------------
    let mut desktop_volume = None;
    let mut mic_volume = None;
    let mut audio_index = 0u32;
    for (branch, role) in [
        (spec.desktop_audio.as_ref(), "desktop"),
        (spec.mic_audio.as_ref(), "mic"),
    ] {
        let Some(branch) = branch else { continue };
        let (chain, volume) = audio_chain(branch)?;
        pipeline.add_many(chain.iter()).map_err(|err| {
            AppError::Capture(format!("could not build the audio pipeline: {err}"))
        })?;
        gst::Element::link_many(chain.iter())
            .map_err(|err| AppError::Capture(format!("could not link the {role} audio: {err}")))?;
        let pad_name = format!("audio_{audio_index}");
        let pad = sink
            .request_pad_simple(&pad_name)
            .ok_or_else(|| AppError::Capture(format!("splitmuxsink has no {pad_name} pad")))?;
        chain
            .last()
            .and_then(|element| element.static_pad("src"))
            .ok_or_else(|| AppError::Capture(format!("{role} queue has no source pad")))?
            .link(&pad)
            .map_err(|err| AppError::Capture(format!("could not link {role} audio: {err:?}")))?;
        match role {
            "desktop" => desktop_volume = Some(volume),
            _ => mic_volume = Some(volume),
        }
        audio_index += 1;
    }

    Ok(Built {
        pipeline,
        sink,
        desktop_volume,
        mic_volume,
        info: PipelineInfo {
            encoder_label: encoder.label.to_string(),
            hardware_encoding: encoder.hardware,
        },
    })
}

fn audio_chain(branch: &AudioBranch) -> Result<(Vec<gst::Element>, gst::Element)> {
    let source = match &branch.source {
        AudioSource::Pulse { device } => {
            let source = element("pulsesrc")?;
            if let Some(device) = device {
                source.set_property("device", device.as_str());
            }
            source
        }
        AudioSource::TestTone { freq } => {
            let source = element("audiotestsrc")?;
            source.set_property("is-live", true);
            source.set_property_from_str("wave", "sine");
            source.set_property("freq", *freq);
            source.set_property("samplesperbuffer", 1024i32);
            source
        }
    };

    let volume = element("volume")?;
    volume.set_property("volume", branch.volume as f64);

    let convert = element("audioconvert")?;
    let resample = element("audioresample")?;
    let caps = capsfilter(
        gst::Caps::builder("audio/x-raw")
            .field("format", "F32LE")
            .field("layout", "interleaved")
            .field("rate", 48_000i32)
            .field("channels", 2i32)
            .build(),
    )?;
    let encoder = element("avenc_aac")?;
    encoder.set_property("bitrate", 192_000i32);
    let queue = element("queue")?;

    Ok((
        vec![
            source,
            volume.clone(),
            convert,
            resample,
            caps,
            encoder,
            queue,
        ],
        volume,
    ))
}

fn element(factory: &str) -> Result<gst::Element> {
    gst::ElementFactory::make(factory).build().map_err(|err| {
        AppError::Capture(format!(
            "the GStreamer element `{factory}` is not available: {err}"
        ))
    })
}

fn capsfilter(caps: gst::Caps) -> Result<gst::Element> {
    let filter = element("capsfilter")?;
    filter.set_property("caps", &caps);
    Ok(filter)
}

struct Encoder {
    factory: &'static str,
    label: &'static str,
    hardware: bool,
}

fn choose_encoder(kind: EncoderKind) -> Result<Encoder> {
    let nvenc = match kind {
        EncoderKind::X264 => false,
        EncoderKind::Auto | EncoderKind::Nvenc => {
            gst::ElementFactory::find("nvh264enc").is_some() && probe_encoder("nvh264enc")
        }
    };
    if nvenc {
        return Ok(Encoder {
            factory: "nvh264enc",
            label: "NVENC H.264 (hardware)",
            hardware: true,
        });
    }
    if gst::ElementFactory::find("x264enc").is_some() {
        if kind == EncoderKind::Nvenc {
            log::warn!("NVENC is not usable, falling back to x264");
        }
        return Ok(Encoder {
            factory: "x264enc",
            label: "x264 (software)",
            hardware: false,
        });
    }
    Err(AppError::Capture(
        "neither nvh264enc nor x264enc is installed".into(),
    ))
}

/// Quick headless encode to prove the encoder can actually open its device.
fn probe_encoder(factory: &str) -> bool {
    let build = || -> Result<gst::Pipeline> {
        let pipeline = gst::Pipeline::new();
        let source = element("videotestsrc")?;
        source.set_property("num-buffers", 3i32);
        let convert = element("videoconvert")?;
        let caps = capsfilter(
            gst::Caps::builder("video/x-raw")
                .field("format", "NV12")
                .build(),
        )?;
        let encoder = element(factory)?;
        let sink = element("fakesink")?;
        sink.set_property("sync", false);
        pipeline
            .add_many([&source, &convert, &caps, &encoder, &sink])
            .map_err(|err| AppError::Capture(format!("encoder probe: {err}")))?;
        gst::Element::link_many([&source, &convert, &caps, &encoder, &sink])
            .map_err(|err| AppError::Capture(format!("encoder probe: {err}")))?;
        Ok(pipeline)
    };

    let Ok(pipeline) = build() else {
        return false;
    };
    if pipeline.set_state(gst::State::Playing).is_err() {
        let _ = pipeline.set_state(gst::State::Null);
        return false;
    }

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut ok = false;
    if let Some(bus) = pipeline.bus() {
        while Instant::now() < deadline {
            let Some(message) = bus.timed_pop(gst::ClockTime::from_mseconds(100)) else {
                continue;
            };
            match message.view() {
                gst::MessageView::Eos(_) => {
                    ok = true;
                    break;
                }
                gst::MessageView::Error(err) => {
                    log::warn!("{factory} is not usable: {}", err.error());
                    break;
                }
                _ => {}
            }
        }
    }

    let _ = pipeline.set_state(gst::State::Null);
    let _ = pipeline.state(gst::ClockTime::from_seconds(5));
    ok
}

/// Pick an even output size, preserving the source aspect ratio, capped at
/// `max_width`.
fn scale_target(size: Option<(i32, i32)>, max_width: u32) -> Option<(i32, i32)> {
    let (width, height) = size?;
    if width <= 0 || height <= 0 {
        return None;
    }
    let max_width = max_width as i32;
    if max_width == 0 || width <= max_width {
        return Some((width, height));
    }
    let scaled_width = (max_width & !1).max(2);
    let scaled_height =
        (((height as f64 * scaled_width as f64 / width as f64).round() as i32) & !1).max(2);
    Some((scaled_width, scaled_height))
}

#[cfg(test)]
mod tests {
    use super::scale_target;

    #[test]
    fn keeps_native_when_small_enough() {
        assert_eq!(scale_target(Some((1280, 720)), 1920), Some((1280, 720)));
        assert_eq!(scale_target(Some((1280, 720)), 0), Some((1280, 720)));
    }

    #[test]
    fn scales_with_aspect_ratio() {
        assert_eq!(scale_target(Some((2560, 1440)), 1920), Some((1920, 1080)));
        assert_eq!(scale_target(Some((3440, 1440)), 2560), Some((2560, 1072)));
        assert_eq!(scale_target(Some((3840, 2160)), 1921), Some((1920, 1080)));
    }

    #[test]
    fn missing_size_means_native() {
        assert_eq!(scale_target(None, 1920), None);
    }
}
