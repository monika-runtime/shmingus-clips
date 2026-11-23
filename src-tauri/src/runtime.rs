//! Runtime environment repairs for the bundled AppImage.
//!
//! Two AppImage-specific problems are fixed here, both before GStreamer or the
//! portals are used:
//!
//! 1. The AppImage launcher sets `GST_PLUGIN_SYSTEM_PATH*` to the AppDir, which
//!    ships GStreamer core libraries but no plugins. Append the host's plugin
//!    directory (and scanner) so capture and encoding work.
//! 2. XDG portals identify a host app by its systemd scope
//!    (`app-<app-id>-<random>.scope`) plus an installed `<app-id>.desktop`.
//!    A directly executed AppImage has neither, so KDE refuses GlobalShortcuts
//!    ("An app id is required") and attributes ScreenCast to the parent app.
//!    Register a desktop entry pointing at the AppImage and, when needed,
//!    re-exec inside a `clipforge` app scope.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Fix GStreamer discovery and portal identity for an AppImage. Safe to call
/// more than once; a no-op outside an AppImage.
pub fn prepare() {
    let Some(appimage) = std::env::var_os("APPIMAGE").map(PathBuf::from) else {
        return;
    };
    if !is_own_appimage(&appimage) {
        // A development binary launched from an agent harness inherits the
        // harness's APPIMAGE; repairing/re-execing that would launch the wrong
        // application (and overwrite our desktop entry). Only act for our own
        // bundle.
        return;
    }

    fix_gstreamer();
    ensure_desktop_entry(&appimage);
    maybe_reexec_in_app_scope(&appimage);
}

/// True when this process really runs from Shmingus Clips' own AppImage: either
/// the executable lives inside an AppImage mount, or the bundle is named after
/// the app (the bundled binary is still `clipforge`; the display name is now
/// "Shmingus Clips").
fn is_own_appimage(appimage: &Path) -> bool {
    let exe_in_mount = std::env::current_exe()
        .map(|exe| exe.to_string_lossy().contains(".mount_"))
        .unwrap_or(false);
    let named_app = appimage
        .file_name()
        .map(|name| {
            let name = name.to_string_lossy().to_lowercase();
            name.contains("clipforge") || name.contains("shmingus")
        })
        .unwrap_or(false);
    exe_in_mount || named_app
}

// ---------------------------------------------------------------------------
// GStreamer plugins
// ---------------------------------------------------------------------------

fn fix_gstreamer() {
    let Some(plugin_dir) = host_plugin_dir() else {
        log::warn!(
            "AppImage: no host GStreamer plugin directory found; capture will be unavailable"
        );
        return;
    };
    let dir = plugin_dir.to_string_lossy().to_string();

    for var in ["GST_PLUGIN_SYSTEM_PATH", "GST_PLUGIN_SYSTEM_PATH_1_0"] {
        let current = std::env::var(var).unwrap_or_default();
        if current.split(':').any(|entry| entry == dir) {
            continue;
        }
        let next = if current.trim_matches(':').is_empty() {
            dir.clone()
        } else {
            format!("{current}:{dir}")
        };
        // SAFETY: called once from main before any thread is spawned.
        unsafe { std::env::set_var(var, next) };
    }

    let scanner = plugin_dir.join("gst-plugin-scanner");
    if scanner.is_file() {
        // SAFETY: as above; the environment is not observed concurrently yet.
        unsafe { std::env::set_var("GST_PLUGIN_SCANNER", &scanner) };
    }

    log::info!("AppImage runtime: using host GStreamer plugins from {dir}");
}

/// First host directory that actually contains the core elements plugin.
fn host_plugin_dir() -> Option<PathBuf> {
    const CANDIDATES: [&str; 4] = [
        "/usr/lib/gstreamer-1.0",
        "/usr/lib64/gstreamer-1.0",
        "/usr/lib/x86_64-linux-gnu/gstreamer-1.0",
        "/usr/lib/i386-linux-gnu/gstreamer-1.0",
    ];

    CANDIDATES.iter().map(PathBuf::from).find(|dir| {
        dir.join("libgstcoreelements.so").exists() || dir.join("libgstcoreelements.so.0").exists()
    })
}

// ---------------------------------------------------------------------------
// Portal identity
// ---------------------------------------------------------------------------

// The internal app id, desktop file name and marker stay `clipforge` so portal
// identity, shortcut bindings and existing desktop entries survive the
// display-name change; only the human-readable Name below is rebranded.
const APP_ID: &str = "clipforge";
const DESKTOP_MARKER: &str = "X-Clipforge-AppImage=";

/// Register `~/.local/share/applications/clipforge.desktop` pointing at this
/// AppImage. The portal needs a loadable desktop entry to accept the app id;
/// GLib rejects an entry whose `Exec` cannot be resolved, so the path must be
/// absolute.
fn ensure_desktop_entry(appimage: &Path) {
    if system_desktop_entry_exists() {
        return;
    }

    let Some(data_home) = user_data_home() else {
        return;
    };
    let applications = data_home.join("applications");
    let target = applications.join(format!("{APP_ID}.desktop"));

    if target.is_file() {
        match std::fs::read_to_string(&target) {
            // Ours: keep it in step with the AppImage's current location.
            Ok(existing) if existing.contains(DESKTOP_MARKER) => {}
            // Someone else's entry: do not touch it.
            Ok(_) => return,
            Err(_) => return,
        }
    }

    if let Err(err) = std::fs::create_dir_all(&applications) {
        log::warn!("could not create {}: {err}", applications.display());
        return;
    }

    let exec = appimage.to_string_lossy();
    let quoted = format!("\"{}\"", exec.replace('"', "\\\""));
    let entry = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Version=1.0\n\
         Name=Shmingus Clips\n\
         Comment=Instant game clips on Linux\n\
         Exec={quoted} %U\n\
         Icon=clipforge\n\
         Terminal=false\n\
         StartupWMClass=clipforge\n\
         Categories=AudioVideo;Video;\n\
         {DESKTOP_MARKER}{exec}\n"
    );

    if let Err(err) = std::fs::write(&target, entry) {
        log::warn!("could not register the desktop entry: {err}");
        return;
    }
    log::info!("AppImage runtime: registered {}", target.display());
}

fn system_desktop_entry_exists() -> bool {
    let name = format!("{APP_ID}.desktop");
    std::env::var_os("XDG_DATA_DIRS")
        .map(|dirs| {
            dirs.to_string_lossy()
                .split(':')
                .filter(|dir| !dir.is_empty())
                .any(|dir| Path::new(dir).join("applications").join(&name).is_file())
        })
        .unwrap_or(false)
}

fn user_data_home() -> Option<PathBuf> {
    dirs::data_dir().or_else(|| dirs::home_dir().map(|home| home.join(".local/share")))
}

/// XDG portals only accept a host app that lives in an `app-<id>-<random>`
/// systemd scope. A directly launched AppImage is in the session/launcher
/// scope instead, so move ourselves into a `clipforge` app scope and hand over.
fn maybe_reexec_in_app_scope(appimage: &Path) {
    if std::env::var_os("CLIPFORGE_NO_REEXEC").is_some() {
        return;
    }
    // Diagnostics and the headless selftest never touch a portal.
    if std::env::args()
        .skip(1)
        .any(|arg| arg == "--doctor" || arg == "--selftest")
    {
        return;
    }
    if current_scope_is_clipforge() {
        return;
    }

    let systemd_run = "/usr/bin/systemd-run";
    if !Path::new(systemd_run).is_file() {
        log::warn!("systemd-run is missing; portals will not accept this app's id");
        return;
    }

    let unit = format!("app-{APP_ID}-{}.scope", std::process::id());
    log::info!("AppImage runtime: re-exec inside {unit}");

    let status = Command::new(systemd_run)
        .args(["--user", "--scope", "--quiet", "--collect"])
        .arg(format!("--unit={unit}"))
        .arg("--")
        .arg(appimage)
        .args(std::env::args().skip(1))
        .status();

    match status {
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(err) => log::warn!("could not re-exec inside a systemd app scope: {err}"),
    }
}

fn current_scope_is_clipforge() -> bool {
    let Ok(cgroup) = std::fs::read_to_string("/proc/self/cgroup") else {
        return false;
    };
    cgroup
        .lines()
        .any(|line| line.contains(&format!("app-{APP_ID}-")))
}
