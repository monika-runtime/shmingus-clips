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
//!    re-exec inside a `shmingus-clips` app scope.

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

    fix_webkit_renderer();
    drop_bundled_python_env();
    prefer_host_libraries();
    fix_gstreamer();
    remove_legacy_desktop_entry();
    install_icons();
    ensure_desktop_entry(&appimage);
    maybe_reexec_in_app_scope(&appimage);
}

// ---------------------------------------------------------------------------
// WebKit on NVIDIA Wayland
// ---------------------------------------------------------------------------

/// WebKitGTK's DMA-BUF renderer does not survive the proprietary NVIDIA driver
/// on some Wayland setups: the compositor drops the connection and the app dies
/// with `Gdk-Message: Error 71 (Protocol error) dispatching to Wayland display`
/// moments after the window appears. Turning the renderer off is the fix, but
/// it costs GPU acceleration for every frame, so it is opt-in rather than
/// applied to every NVIDIA machine.
fn fix_webkit_renderer() {
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_some() {
        return;
    }
    if !crate::settings::software_rendering_requested() {
        return;
    }
    // SAFETY: called once from main before any thread is spawned.
    unsafe { std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1") };
    log::info!("AppImage runtime: software rendering requested, disabling WebKit's DMA-BUF renderer");
}

/// The bundle ships no Python, but a host GStreamer plugin can start the
/// system `python3` with the AppImage's `PYTHONHOME`, which then aborts with
/// "No module named 'encodings'". Drop those variables when they point into the
/// bundle so the host Python uses its own paths.
fn drop_bundled_python_env() {
    let appdir = std::env::var("APPDIR").unwrap_or_default();
    for var in ["PYTHONHOME", "PYTHONPATH"] {
        let Some(value) = std::env::var_os(var) else {
            continue;
        };
        let text = value.to_string_lossy();
        let inside_bundle = text.contains(".mount_")
            || (!appdir.is_empty() && text.starts_with(appdir.as_str()));
        if inside_bundle {
            // SAFETY: called once from main before any thread is spawned.
            unsafe { std::env::remove_var(var) };
        }
    }
}

// ---------------------------------------------------------------------------
// Startup crash guard
// ---------------------------------------------------------------------------

/// Counts startup attempts and remembers the launch chain, so an in-app update
/// relaunch or a portal re-exec is not mistaken for a fresh launch.
const STARTUP_COUNT: &str = "startup-crashes";
const STARTUP_TOKEN: &str = "startup-token";
const STARTUP_TOKEN_ENV: &str = "SHMINGUS_CLIPS_STARTUP_TOKEN";
const COMPAT_AUTO_NOTE: &str = "compatibility-auto-enabled";
/// Failed startups before Compatibility rendering turns itself on. A clean
/// start or exit clears the count, so only a real hard crash counts.
const CRASHES_BEFORE_COMPAT: u32 = 1;

/// Call before the webview starts. If the previous launches died during
/// startup, switch to Compatibility rendering; otherwise record this attempt.
/// A launch that reaches the running state calls [`startup_guard_commit`].
pub fn startup_guard_begin() {
    if std::env::var_os("APPIMAGE").is_none() || !is_own_appimage_for_guard() {
        return;
    }
    if std::env::args()
        .skip(1)
        .any(|arg| arg == "--selftest" || arg == "--doctor" || arg == "--no-software-rendering")
    {
        return;
    }
    if crate::settings::software_rendering_requested() {
        return;
    }

    let cache = crate::settings::cache_dir();
    let token_path = cache.join(STARTUP_TOKEN);
    if same_launch_chain(&token_path) {
        return;
    }

    let token = uuid::Uuid::new_v4().to_string();
    if std::fs::create_dir_all(&cache).is_ok() {
        let _ = std::fs::write(&token_path, &token);
    }
    // SAFETY: called from main before any thread is spawned.
    unsafe { std::env::set_var(STARTUP_TOKEN_ENV, &token) };

    let count_path = cache.join(STARTUP_COUNT);
    let previous: u32 = std::fs::read_to_string(&count_path)
        .ok()
        .and_then(|raw| raw.trim().parse().ok())
        .unwrap_or(0);

    if previous >= CRASHES_BEFORE_COMPAT {
        match crate::settings::enable_software_rendering() {
            Ok(()) => {
                log::warn!(
                    "startup crashed {previous} times; enabling Compatibility rendering"
                );
                let _ = std::fs::write(cache.join(COMPAT_AUTO_NOTE), "1");
                let _ = std::fs::remove_file(&count_path);
                let _ = std::fs::remove_file(&token_path);
            }
            Err(err) => log::warn!("could not save the Compatibility rendering choice: {err}"),
        }
        return;
    }

    let _ = std::fs::write(count_path, (previous + 1).to_string());
}

/// The window has been up long enough (or the app is exiting): this launch is
/// not a startup crash.
pub fn startup_guard_commit() {
    let cache = crate::settings::cache_dir();
    let _ = std::fs::remove_file(cache.join(STARTUP_COUNT));
    let _ = std::fs::remove_file(cache.join(STARTUP_TOKEN));
    // SAFETY: only removes a variable this process may have set.
    unsafe { std::env::remove_var(STARTUP_TOKEN_ENV) };
}

/// Whether Compatibility rendering was switched on automatically, so the
/// interface can say so once.
pub fn take_auto_compat_note() -> bool {
    let path = crate::settings::cache_dir().join(COMPAT_AUTO_NOTE);
    if path.is_file() {
        let _ = std::fs::remove_file(path);
        true
    } else {
        false
    }
}

fn same_launch_chain(token_path: &Path) -> bool {
    let Ok(token) = std::env::var(STARTUP_TOKEN_ENV) else {
        return false;
    };
    std::fs::read_to_string(token_path)
        .map(|stored| stored.trim() == token)
        .unwrap_or(false)
}

/// True when this binary runs from our own AppImage. Mirrors the check in
/// `prepare` without needing the path again.
fn is_own_appimage_for_guard() -> bool {
    std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .map(|path| is_own_appimage(&path))
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Host vs bundled libraries
// ---------------------------------------------------------------------------

/// Library directories a host keeps its GStreamer stack in.
const HOST_LIB_DIRS: [&str; 4] = [
    "/usr/lib/x86_64-linux-gnu",
    "/usr/lib64",
    "/usr/lib",
    "/usr/lib/i386-linux-gnu",
];

/// Holds the pid of the process that already moved the host libraries first,
/// so the re-exec cannot loop. A pid rather than a flag: the relauched process
/// after an in-app update inherits the environment, and it must run the repair
/// again rather than skip it.
const HOST_LIBS_MARKER: &str = "SHMINGUS_CLIPS_HOST_LIBS";

/// Put the host's GStreamer and GLib ahead of the copies bundled in the
/// AppImage.
///
/// The bundle carries the build machine's GStreamer and GLib (1.20 / 2.72 on
/// Ubuntu 22.04), while the plugins come from the host. A host plugin built
/// against a newer GLib then fails to load - on Arch the scanner reports
/// `undefined symbol: g_once_init_leave_pointer` - and the app reports missing
/// elements such as the encoders. Running the whole stack from the host keeps
/// core, libraries and plugins on one version; the bundled copies stay behind
/// it as a fallback for libraries the host does not have.
fn prefer_host_libraries() {
    let pid = std::process::id().to_string();
    if std::env::var(HOST_LIBS_MARKER).ok().as_deref() == Some(pid.as_str()) {
        return;
    }

    let host: Vec<String> = HOST_LIB_DIRS
        .iter()
        .filter(|dir| Path::new(dir).join("libgstreamer-1.0.so.0").is_file())
        .map(|dir| (*dir).to_string())
        .collect();
    if host.is_empty() {
        log::warn!(
            "AppImage runtime: no host GStreamer library found; keeping the bundled one"
        );
        return;
    }

    let current = std::env::var("LD_LIBRARY_PATH").unwrap_or_default();
    let mut dirs = host.clone();
    dirs.extend(
        current
            .split(':')
            .filter(|entry| !entry.is_empty() && !host.iter().any(|dir| dir == entry))
            .map(str::to_string),
    );

    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    log::info!(
        "AppImage runtime: preferring host libraries from {}",
        host.join(":")
    );

    use std::os::unix::process::CommandExt;
    let err = Command::new(&exe)
        .args(std::env::args_os().skip(1))
        .env("LD_LIBRARY_PATH", dirs.join(":"))
        .env(HOST_LIBS_MARKER, &pid)
        .exec();
    log::warn!("could not re-exec with the host libraries ({err}); keeping the bundled ones");
}

/// True when this process really runs from Shmingus Clips' own AppImage: either
/// the executable lives inside an AppImage mount, or the bundle is named after
/// the app (the bundled binary is still `shmingus-clips`; the display name is now
/// "Shmingus Clips").
fn is_own_appimage(appimage: &Path) -> bool {
    let exe_in_mount = std::env::current_exe()
        .map(|exe| exe.to_string_lossy().contains(".mount_"))
        .unwrap_or(false);
    let named_app = appimage
        .file_name()
        .map(|name| {
            let name = name.to_string_lossy().to_lowercase();
            name.contains("shmingus-clips") || name.contains("shmingus")
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

// The internal app id and desktop file name. It must not contain a hyphen:
// xdg-desktop-portal derives the app id from the systemd scope with
// `app-(?:[[:alnum:]]+-)?(.+?)(?:-[[:alnum:]]*)\.scope`, so
// `app-shmingus-clips-123.scope` would resolve to `clips` and the portal would
// look for a desktop file that does not exist (no prompt, no hotkeys).
const APP_ID: &str = "shmingus";
const DESKTOP_MARKER: &str = "X-Shmingus-Clips-AppImage=";
/// Launcher entries left behind by earlier names, removed on sight.
const LEGACY_ENTRIES: [(&str, &str); 2] = [
    ("clipforge", "X-Clipforge-AppImage="),
    ("shmingus-clips", "X-Shmingus-Clips-AppImage="),
];

/// Register `~/.local/share/applications/shmingus-clips.desktop` pointing at this
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
    let exec = appimage.to_string_lossy();

    if target.is_file() {
        match std::fs::read_to_string(&target) {
            // Ours: leave it alone only while it still points at this AppImage
            // (a newer download or a moved file must refresh the entry).
            Ok(existing)
                if existing.contains(DESKTOP_MARKER)
                    && existing.contains(&format!("{DESKTOP_MARKER}{exec}")) =>
            {
                return;
            }
            // Someone else's entry: do not touch it.
            Ok(existing) if !existing.contains(DESKTOP_MARKER) => return,
            Ok(_) => {}
            Err(_) => return,
        }
    }

    if let Err(err) = std::fs::create_dir_all(&applications) {
        log::warn!("could not create {}: {err}", applications.display());
        return;
    }

    let quoted = format!("\"{}\"", exec.replace('"', "\\\""));
    let entry = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Version=1.0\n\
         Name=Shmingus Clips\n\
         Comment=Instant game clips on Linux\n\
         Exec={quoted} %U\n\
         Icon={APP_ID}\n\
         Terminal=false\n\
         StartupWMClass={APP_ID}\n\
         Categories=AudioVideo;Video;\n\
         {DESKTOP_MARKER}{exec}\n"
    );

    if let Err(err) = std::fs::write(&target, entry) {
        log::warn!("could not register the desktop entry: {err}");
        return;
    }
    log::info!("AppImage runtime: registered {}", target.display());
}

/// Copy the bundle's launcher icons into the user's icon theme, so Plasma can
/// resolve the desktop entry's `Icon=shmingus` after the AppImage unmounts.
/// Without this the launcher and taskbar fall back to a generic icon.
fn install_icons() {
    let Some(appdir) = app_dir() else {
        return;
    };
    let Some(data_home) = user_data_home() else {
        return;
    };
    let source_root = appdir.join("usr/share/icons/hicolor");
    let mut installed = 0;
    for size in ["16x16", "32x32", "64x64", "128x128", "256x256", "256x256@2"] {
        let source = ["shmingus-clips.png", "shmingus.png", "Shmingus Clips.png"]
            .iter()
            .map(|name| source_root.join(size).join("apps").join(name))
            .find(|path| path.is_file());
        let Some(source) = source else {
            continue;
        };
        let target_dir = data_home.join("icons/hicolor").join(size).join("apps");
        if std::fs::create_dir_all(&target_dir).is_err() {
            continue;
        }
        match std::fs::copy(&source, target_dir.join(format!("{APP_ID}.png"))) {
            Ok(_) => installed += 1,
            Err(err) => log::warn!("could not install the {size} app icon: {err}"),
        }
    }
    if installed > 0 {
        log::info!("AppImage runtime: installed {installed} launcher icon(s)");
    }
}

/// The mounted AppImage root: `<appdir>/usr/bin/shmingus-clips` is the binary.
fn app_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("APPDIR") {
        return Some(PathBuf::from(dir));
    }
    let exe = std::env::current_exe().ok()?;
    exe.parent()?.parent()?.parent().map(PathBuf::from)
}

/// Drop launcher entries written under earlier app ids. Only touches a file
/// carrying our marker, never someone else's entry.
fn remove_legacy_desktop_entry() {
    let Some(home) = user_data_home() else {
        return;
    };
    for (app_id, marker) in LEGACY_ENTRIES {
        let path = home.join("applications").join(format!("{app_id}.desktop"));
        let Ok(existing) = std::fs::read_to_string(&path) else {
            continue;
        };
        if existing.contains(marker) {
            let _ = std::fs::remove_file(&path);
            log::info!("AppImage runtime: removed the old {app_id} launcher entry");
        }
    }
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
/// scope instead, so move ourselves into a `shmingus-clips` app scope and hand over.
fn maybe_reexec_in_app_scope(appimage: &Path) {
    if std::env::var_os("SHMINGUS_CLIPS_NO_REEXEC").is_some() {
        return;
    }
    // Diagnostics and the headless selftest never touch a portal.
    if std::env::args()
        .skip(1)
        .any(|arg| arg == "--doctor" || arg == "--selftest")
    {
        return;
    }
    if current_scope_is_ours() {
        return;
    }

    let systemd_run = "/usr/bin/systemd-run";
    if !Path::new(systemd_run).is_file() {
        log::warn!("systemd-run is missing; portals will not accept this app's id");
        return;
    }

    let unit = format!("app-{APP_ID}-{}.scope", std::process::id());
    log::info!("AppImage runtime: re-exec inside {unit}");

    // Launch the inner binary rather than the AppImage: the AppRun wrapper
    // prepends the bundle's library directory, which would undo the host-first
    // order set by `prefer_host_libraries`. Portal identity comes from the
    // systemd scope name and the desktop entry, not from the executable path.
    let exe = std::env::current_exe().unwrap_or_else(|_| appimage.to_path_buf());

    let status = Command::new(systemd_run)
        .args(["--user", "--scope", "--quiet", "--collect"])
        .arg(format!("--unit={unit}"))
        .arg("--")
        .arg(exe)
        .args(std::env::args().skip(1))
        .status();

    match status {
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(err) => log::warn!("could not re-exec inside a systemd app scope: {err}"),
    }
}

fn current_scope_is_ours() -> bool {
    let Ok(cgroup) = std::fs::read_to_string("/proc/self/cgroup") else {
        return false;
    };
    cgroup
        .lines()
        .any(|line| line.contains(&format!("app-{APP_ID}-")))
}
