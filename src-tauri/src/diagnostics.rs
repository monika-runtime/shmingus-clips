use std::process::Command;

use crate::error::Result;
use crate::model::{CheckResult, EnvironmentReport, Settings, SelfTestReport};

fn command_ok(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn portal_property(interface: &str, property: &str) -> Option<String> {
    command_output(
        "gdbus",
        &[
            "call",
            "--session",
            "--dest",
            "org.freedesktop.portal.Desktop",
            "--object-path",
            "/org/freedesktop/portal/desktop",
            "--method",
            "org.freedesktop.DBus.Properties.Get",
            interface,
            property,
        ],
    )
}

/// Portal `version` property as a plain number. gdbus answers with something
/// like `(<uint32 6>,)`; that belongs in a debug log, not on screen.
fn portal_version(interface: &str) -> Option<u32> {
    let raw = portal_property(interface, "version")?;
    let start = raw.find("uint32")? + "uint32".len();
    raw[start..]
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()
}

fn check(
    checks: &mut Vec<CheckResult>,
    id: &str,
    label: &str,
    required: bool,
    ok: bool,
    detail: impl Into<String>,
) {
    checks.push(CheckResult {
        id: id.to_string(),
        label: label.to_string(),
        required,
        ok,
        detail: detail.into(),
    });
}

/// Plain-text snapshot for a bug report: versions, session, GPU, portal
/// checks and the non-secret settings. Never includes tokens.
pub fn bug_report(settings: &Settings) -> String {
    let mut out = String::new();
    let os = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|text| {
            text.lines()
                .find_map(|line| line.strip_prefix("PRETTY_NAME="))
                .map(|value| value.trim_matches('"').to_string())
        })
        .unwrap_or_else(|| "unknown".into());
    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".into());
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "unknown".into());
    let appimage = std::env::var("APPIMAGE").unwrap_or_else(|_| "no".into());
    let compat_env = std::env::var("WEBKIT_DISABLE_DMABUF_RENDERER").ok();

    out.push_str("Shmingus Clips bug report\n");
    out.push_str(&format!("generated: {}\n", chrono::Utc::now().to_rfc3339()));
    out.push_str("\n[app]\n");
    out.push_str(&format!("version: {}\n", env!("CARGO_PKG_VERSION")));
    out.push_str(&format!("bundle: net.shmingus.clips\n"));
    out.push_str(&format!("appimage: {appimage}\n"));
    out.push_str(&format!(
        "compatibility rendering: setting={} env={}\n",
        settings.software_rendering,
        compat_env.as_deref().unwrap_or("unset")
    ));

    out.push_str("\n[system]\n");
    out.push_str(&format!("os: {os}\n"));
    out.push_str(&format!(
        "kernel: {}\n",
        command_output("uname", &["-r"]).unwrap_or_else(|| "unknown".into())
    ));
    out.push_str(&format!("session: {session} / {desktop}\n"));
    out.push_str(&format!(
        "wayland display: {}\n",
        if std::env::var_os("WAYLAND_DISPLAY").is_some() { "yes" } else { "no" }
    ));
    if let Some(lspci) = command_output("lspci", &["-nn"]) {
        let gpus: Vec<&str> = lspci
            .lines()
            .filter(|line| line.contains("VGA compatible") || line.contains("3D controller"))
            .collect();
        out.push_str(&format!("gpu: {}\n", if gpus.is_empty() { "unknown".into() } else { gpus.join(" | ") }));
    }
    if let Some(info) = command_output("pactl", &["info"]) {
        let server = info
            .lines()
            .find_map(|line| line.strip_prefix("Server Name:"))
            .map(str::trim)
            .unwrap_or("unknown");
        out.push_str(&format!("audio server: {server}\n"));
    }
    for (label, program, arg) in [
        ("ffmpeg", "ffmpeg", "-version"),
        ("ffprobe", "ffprobe", "-version"),
        ("gstreamer", "gst-inspect-1.0", "--version"),
    ] {
        let version = command_output(program, &[arg])
            .and_then(|text| text.lines().next().map(str::to_string))
            .unwrap_or_else(|| "not found".into());
        out.push_str(&format!("{label}: {version}\n"));
    }

    out.push_str("\n[environment]\n");
    for check in environment_report().checks {
        out.push_str(&format!(
            "{} {}: {}\n",
            if check.ok { "[ok]" } else { "[missing]" },
            check.label,
            check.detail
        ));
    }

    out.push_str("\n[settings]\n");
    out.push_str(&format!(
        "buffer: {}s, {} fps, {} Mbps, encoder {:?}, max width {}\n",
        settings.buffer_seconds, settings.fps, settings.bitrate_mbps, settings.encoder, settings.max_width
    ));
    out.push_str(&format!(
        "audio: desktop={} {:.0}%, mic={} {:.0}%, device={}\n",
        settings.audio.desktop_enabled,
        settings.audio.desktop_volume * 100.0,
        settings.audio.mic_enabled,
        settings.audio.mic_volume * 100.0,
        settings.audio.mic_device.as_deref().unwrap_or("system default")
    ));
    out.push_str(&format!(
        "capture: {}\n",
        settings
            .capture_monitor
            .as_ref()
            .map(|monitor| monitor.name.as_str())
            .unwrap_or("not selected")
    ));
    out.push_str(&format!(
        "hotkeys: save={}, buffer={}, mic={}\n",
        settings.hotkeys.save_clip, settings.hotkeys.toggle_buffer, settings.hotkeys.toggle_mic
    ));
    out.push_str(&format!(
        "share server: {}\n",
        if settings.share_base_url.trim().is_empty() {
            "off"
        } else {
            settings.share_base_url.trim()
        }
    ));
    out.push_str("\nNo tokens or passwords are included.\n");
    out
}

/// Full environment report used by the setup screen and the `--doctor` flag.
pub fn environment_report() -> EnvironmentReport {
    let mut checks = Vec::new();

    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".into());
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "unknown".into());
    let wayland = session == "wayland" || std::env::var("WAYLAND_DISPLAY").is_ok();

    check(
        &mut checks,
        "session",
        "Desktop session",
        false,
        wayland,
        if wayland {
            format!("{session} ({desktop})")
        } else {
            format!("{session} ({desktop}) - screen capture needs a Wayland session")
        },
    );

    let screencast = portal_version("org.freedesktop.portal.ScreenCast");
    check(
        &mut checks,
        "portal-screencast",
        "Screen recording",
        true,
        screencast.is_some(),
        screencast
            .map(|v| format!("available (version {v})"))
            .unwrap_or_else(|| "screen-sharing support is not reachable".into()),
    );

    let shortcuts = portal_version("org.freedesktop.portal.GlobalShortcuts");
    check(
        &mut checks,
        "portal-shortcuts",
        "Global hotkeys",
        false,
        shortcuts.is_some(),
        shortcuts
            .map(|v| format!("available (version {v})"))
            .unwrap_or_else(|| "not available - the in-app buttons still work".into()),
    );

    let pipewire = command_ok("pgrep", &["-x", "pipewire"]);
    check(
        &mut checks,
        "pipewire",
        "Audio and video server",
        true,
        pipewire,
        if pipewire {
            "PipeWire is running".to_string()
        } else {
            "PipeWire is not running".to_string()
        },
    );

    let ffmpeg =
        command_output("ffmpeg", &["-version"]).and_then(|v| v.lines().next().map(str::to_string));
    check(
        &mut checks,
        "ffmpeg",
        "Video tools",
        true,
        ffmpeg.is_some(),
        ffmpeg.unwrap_or_else(|| "ffmpeg not found in PATH".into()),
    );

    let ffprobe = crate::media::ffprobe_available();
    check(
        &mut checks,
        "ffprobe",
        "Video inspection",
        true,
        ffprobe,
        if ffprobe {
            "available".to_string()
        } else {
            "ffprobe not found in PATH".to_string()
        },
    );

    for (id, plugin, label, required) in [
        (
            "gst-pipewiresrc",
            "pipewiresrc",
            "Screen capture module",
            true,
        ),
        ("gst-mpegtsmux", "mpegtsmux", "Clip container", true),
        ("gst-splitmuxsink", "splitmuxsink", "Clip splitter", true),
        (
            "gst-nvh264enc",
            "nvh264enc",
            "Hardware video encoder",
            false,
        ),
        ("gst-x264enc", "x264enc", "Software video encoder", true),
    ] {
        let ok = command_ok("gst-inspect-1.0", &[plugin]);
        check(
            &mut checks,
            id,
            label,
            required,
            ok,
            if ok {
                format!("{plugin} present")
            } else {
                format!("{plugin} missing - install the matching GStreamer plugin")
            },
        );
    }

    let mut encoders = Vec::new();
    if command_ok("gst-inspect-1.0", &["nvh264enc"]) {
        encoders.push("NVENC H.264 (hardware)".to_string());
    }
    if command_ok("gst-inspect-1.0", &["x264enc"]) {
        encoders.push("x264 (software)".to_string());
    }

    let ok = checks.iter().all(|c| c.ok || !c.required);

    EnvironmentReport {
        ok,
        checks,
        encoders,
        session_type: session,
        desktop,
    }
}

/// `shmingus-clips --doctor`: print the environment report and exit non-zero when
/// a required dependency is missing.
pub fn doctor() {
    let report = environment_report();
    println!(
        "{}",
        serde_json::to_string_pretty(&report).unwrap_or_default()
    );
    if !report.ok {
        std::process::exit(1);
    }
}

/// `shmingus-clips --selftest`: exercise the capture engine headlessly (no portal
/// consent needed) and print a JSON report.
pub fn selftest() {
    let result: Result<SelfTestReport> = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| crate::error::AppError::msg(format!("tokio runtime: {e}")))
        .and_then(|rt| rt.block_on(crate::capture::selftest()));

    match result {
        Ok(report) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&report).unwrap_or_default()
            );
            if !report.ok {
                std::process::exit(1);
            }
        }
        Err(err) => {
            eprintln!("selftest failed: {err}");
            std::process::exit(1);
        }
    }
}
