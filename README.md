# Shmingus Clips

Instant game clips for Linux. Shmingus Clips keeps a rolling replay buffer of your
screen and audio, and saves the last N seconds to an MP4 the moment you hit a
hotkey - no recording session to start, no cloud account.

Built for KDE Plasma on Wayland (XDG desktop portals), with a Rust backend and a
Tauri 2 + React frontend.

## What it does

- **Replay buffer** - captures one monitor through the ScreenCast portal via
  PipeWire, encodes with NVENC when available (x264 fallback), and keeps 5–300s
  of footage in 1-second MPEG-TS fragments.
- **Two audio tracks** - desktop audio (default sink monitor) and microphone are
  recorded as separate AAC tracks in the saved MP4, each with its own volume.
- **Global hotkey** - saves the last N seconds (`ALT+c` by default) through
  the GlobalShortcuts portal, so it works while a game has focus.
- **Audible confirmation** - a short chime plays when a clip is saved, so you
  know it worked even when a fullscreen game hides notifications (toggle in
  Settings → App).
- **Game detection and auto-capture** - recognises Steam games from `/proc`
  environment + appmanifest files across all Steam libraries (Heroic best
  effort). With auto-start on (default), Shmingus Clips asks for screen permission
  once at launch, starts the buffer when a game launches, saves the buffered
  moments when it exits (a crash looks the same as a quit to the detector),
  and stops. Manual Start/Stop still works and is never auto-stopped.
- **Library** - thumbnails, search, filters, favorites, rename, reveal in file
  manager, delete, and keyframe-accurate trim into a new clip.
- **Themes** - Lime, Violet, Cyan lab and Amber tape, picked in
  Settings → App and applied instantly; the brand mark follows the accent.
- **Sharing** - run the bundled self-hosted service in `share-server/` and
  upload a clip from the player or context menu. You get a permanent link with
  a player page, OpenGraph/Twitter/oEmbed metadata for Discord embeds, Range
  streaming, and a delete button. Uploads are manual; local files stay put.
- **Context menus** - right-click a clip, the player, the recorder card or empty
  space for the relevant actions; text fields keep the normal editing menu.
- **Tray** - open the app, save a clip, start/stop the buffer, or quit without
  leaving the game.
- **No overlay** - Wayland cannot draw a global overlay without compositor
  support; Shmingus Clips notifies via tray + desktop notification instead.

## Requirements

- Wayland session with `xdg-desktop-portal` and a ScreenCast backend
  (`xdg-desktop-portal-kde` or `-gnome`)
- PipeWire with the PulseAudio compatibility layer (`pipewire-pulse`,
  `pulseaudio-utils` for `pactl`)
- GStreamer 1.24+ with `pipewiresrc`, `x264enc`, `mpegtsmux`, `splitmuxsink`
  (plus `nvcodec`/`nvh264enc` for NVIDIA hardware encoding)
- FFmpeg + FFprobe (muxing, trimming, thumbnails)
- WebKitGTK 4.1 and GTK 3 to run the Tauri app

On Arch:

```bash
sudo pacman -S --needed pipewire pipewire-pulse pipewire-audio \
  gst-plugins-base gst-plugins-good gst-plugins-bad gst-plugin-pipewire \
  gst-plugins-ugly gst-libav nvidia-utils ffmpeg webkit2gtk-4.1 \
  libayatana-appindicator xdg-desktop-portal xdg-desktop-portal-kde
```

Check the machine:

```bash
./src-tauri/target/debug/clipforge --doctor     # required dependencies
./src-tauri/target/debug/clipforge --selftest   # headless encode → clip pipeline
```

## Build and run

```bash
pnpm install
pnpm tauri dev                    # development, hot reload
pnpm tauri build --no-bundle      # release binary
pnpm tauri build                  # bundles too; needs dpkg-deb / appimagetool
```

`--no-bundle` produces `src-tauri/target/release/clipforge`, which is the
simplest way to run the app without packaging tools installed.

## First run

1. Open Shmingus Clips, go to **Settings → Capture** and press **Choose screen**.
   KDE asks which monitor to share and remembers it (restore token) next time.
2. Press **Start buffer** (or the tray item). The first start may open the
   screen-share dialog; later starts reuse the authorisation.
3. KDE asks once to approve the global hotkeys. After that `Alt+C` saves a
   clip, `Ctrl+Alt+B` toggles the buffer, `Ctrl+Alt+M` mutes the mic.
   If the shortcuts stay inactive, enable Shmingus Clips in **System Settings →
   Shortcuts** and save the hotkey settings again.
4. Clips land in `~/Videos/Clipforge/<Game>/`.

### Why a monitor and not a window?

On Wayland an application cannot discover or capture another application's
window on its own. The portal owns window selection, so Shmingus Clips records the
monitor you pick; game detection supplies the name and grouping. Fullscreen
games are captured exactly as expected. Per-window capture would require the
user to pick the window through the portal each time.

## Architecture

```
src/                    React UI (hash routes: #/, #/library, #/settings)
  state/store.tsx       app store, event subscriptions, dirty settings
  lib/bridge.ts         Tauri commands in desktop, seed backend in a browser
src-tauri/src/
  capture/              portal ScreenCast, GStreamer pipeline, TS ring, clip saving
  detect/               /proc + Steam/Heroic game detection
  shortcuts.rs          GlobalShortcuts portal bindings
  library/              JSON clip index, thumbnails, trim, export
  tray.rs               tray menu and window toggle
  commands.rs           frozen Tauri command surface
  model.rs              shared serde types (mirror: src/lib/types.ts)
  media.rs              ffmpeg/ffprobe helpers
  diagnostics.rs        --doctor / --selftest + environment report
```

Capture pipeline (one GStreamer thread, no GTK main loop):

```
pipewiresrc ─► videorate ─► videoconvert ─► [videoscale] ─► nvh264enc|x264enc
                                                        ─► h264parse ─► queue ─┐
pulsesrc (sink.monitor) ─► volume ─► audioconvert ─► audioresample ─► avenc_aac ─┤ splitmuxsink
pulsesrc (mic)          ─► volume ─► audioconvert ─► audioresample ─► avenc_aac ─┘  (1s MPEG-TS)
```

Saving a clip asks `splitmuxsink` to split immediately, concatenates the last N
fragments (keeping every stream with `-map 0`), remuxes to MP4 with
`-c copy -movflags +faststart`, and writes a thumbnail.

Thumbnails are served through Tauri's `asset` protocol, but `<video>` cannot use
it: WebKitGTK hands media URLs to GStreamer, which knows nothing about Tauri's
protocol handler. Clips are therefore served by a small loopback HTTP server
with `Range` support, reachable only by library clip id.

## Data locations

| What | Path |
|------|------|
| Settings | `~/.config/clipforge/settings.json` |
| Library index | `~/.config/clipforge/library.json` |
| Thumbnails, temp segments | `~/.cache/clipforge/` |
| Clips | `~/Videos/Clipforge/` (configurable) |
| Exports | `~/Videos/Clipforge/Exports/` |

## Known limitations

- Wayland only; X11 sessions are not supported.
- Monitor capture, not automatic per-window capture (see above).
- No in-game overlay.
- Game detection covers Steam (full) and Heroic (best effort); Lutris is not
  installed on this machine and is not guessed at.
- The replay buffer is held on disk as 1-second TS fragments, so a clip can be
  up to ~1s shorter than the configured window.

## AppImage notes

The AppImage repairs two things the bundle cannot do by itself:

- **GStreamer plugins.** The AppImage runtime points `GST_PLUGIN_SYSTEM_PATH`
  at the AppDir, which ships GStreamer core libraries but no plugins. At
  startup Shmingus Clips appends the host's plugin directory (and plugin scanner),
  so capture and encoding use the GStreamer plugins installed on the system.
- **Portal identity.** XDG portals identify a host app by its systemd app scope
  plus a desktop entry. A directly executed AppImage has neither, so KDE
  refuses global shortcuts and names the parent app in the screen-share prompt.
  Shmingus Clips registers `~/.local/share/applications/clipforge.desktop` (pointing
  at the AppImage) and re-executes itself in an `app-clipforge-*.scope`. The
  first launch asks KDE to approve the shortcuts; approve once and they are
  remembered. If you decline, enable Shmingus Clips later under
  **System Settings → Shortcuts** and save the hotkey settings again.

This needs a systemd user session (standard on KDE Plasma). The AppImage does
not bundle GStreamer plugins; install them as listed under Requirements.


## Sharing

`share-server/` is a standalone Rust service (Axum + SQLite + disk) that turns
clips into links. It is separate from the desktop app:

```bash
cd share-server
cargo run --release            # http://0.0.0.0:8080, data in ./data
# or: docker compose up -d --build
```

Set `SHARE_PUBLIC_URL` to the public HTTPS address (for example
`https://clips.shmingus.net`) before exposing it, put that address in
**Settings → Sharing**, and upload a clip from the player or its right-click
menu. Links stay valid until you remove the share from the app (or delete it
through the API).

**Discord embeds:** paste the `/v/<id>` link into a channel. Discord reads the
OpenGraph video tags and the oEmbed discovery link to build the embed; the
`/embed/<id>` page is there for iframes. Discord only unfurls a public HTTPS
URL, not `localhost` or plain HTTP.

**Uploads are authenticated with Authentik.** The service is provisioned with
the public OIDC client `Clips` on `https://auth.shmingus.net`; set
`SHARE_AUTHENTIK_ISSUER=https://auth.shmingus.net/application/o/clips/` and
`SHARE_AUTHENTIK_CLIENT_ID=` (optionally `SHARE_AUTHENTIK_REQUIRED_GROUP=clips-users`)
and only members of that group can upload.
In Shmingus Clips use **Settings → Sharing → Sign in with Authentik**; the
system browser handles the login (authorization code + PKCE, no password in the
app) and uploads carry the resulting access token. Viewing share links never
requires an account.

The service's README covers every environment variable, the Docker image,
Caddy/Nginx HTTPS, backups and deletion. Uploads and thumbnails live in
`SHARE_DATA_DIR`; the SQLite database is in the same directory.

## Verification

```bash
pnpm typecheck && pnpm build
cargo test --manifest-path src-tauri/Cargo.toml
./src-tauri/target/debug/clipforge --doctor
./src-tauri/target/debug/clipforge --selftest
```

`--selftest` runs the real encoder/segment/remux path with a test pattern and
two tone tracks - no portal consent needed - and asserts a 5–7s MP4 with two
audio streams and a thumbnail.
