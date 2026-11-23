import type {
  AudioDevice,
  AuthStatus,
  Clip,
  EnvironmentReport,
  GameInfo,
  RecorderStatus,
  SavedMonitor,
  SelfTestReport,
  Settings,
  ShareInfo,
  ShareServerStatus,
} from "./types";
import { DEFAULT_HOTKEYS } from "./defaults";

/**
 * Browser preview backend.
 *
 * `pnpm dev` in a plain browser has no Tauri runtime. Instead of invoking
 * commands that cannot exist there, the bridge answers with this seed library
 * and applies mutations in memory so every screen can be reviewed. Nothing
 * here is used inside the desktop app.
 */

const wait = (ms = 180) =>
  new Promise<void>((resolve) => {
    window.setTimeout(resolve, ms);
  });

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as unknown as T;
}

function thumb(from: string, to: string, glow: string): string {
  const holes: string[] = [];
  for (let x = 10; x < 480; x += 36) {
    holes.push(
      `<rect x="${x}" y="9" width="18" height="10" rx="2"/>`,
      `<rect x="${x}" y="251" width="18" height="10" rx="2"/>`,
    );
  }
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 480 270">
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="${from}"/>
      <stop offset="1" stop-color="${to}"/>
    </linearGradient>
    <radialGradient id="glow" cx="0.64" cy="0.32" r="0.72">
      <stop offset="0" stop-color="${glow}" stop-opacity="0.55"/>
      <stop offset="1" stop-color="${glow}" stop-opacity="0"/>
    </radialGradient>
  </defs>
  <rect width="480" height="270" fill="url(#bg)"/>
  <circle cx="300" cy="88" r="160" fill="url(#glow)"/>
  <path d="M0 198 L82 146 L150 186 L228 116 L312 180 L398 134 L480 172 L480 270 L0 270 Z" fill="#07090B" opacity="0.42"/>
  <path d="M0 228 L104 182 L186 214 L272 162 L356 208 L480 172 L480 270 L0 270 Z" fill="#07090B" opacity="0.4"/>
  <g fill="#05070A" opacity="0.72">${holes.join("")}</g>
  <g stroke="#FFFFFF" stroke-opacity="0.045">
    <line x1="120" y1="0" x2="120" y2="270"/>
    <line x1="240" y1="0" x2="240" y2="270"/>
    <line x1="360" y1="0" x2="360" y2="270"/>
  </g>
  <circle cx="42" cy="38" r="4" fill="${glow}" opacity="0.9"/>
</svg>`;
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}

const THUMBS = {
  elden: thumb("#28352B", "#0B110D", "#D8B26A"),
  hades: thumb("#3A1F2A", "#120A0E", "#FF6B8B"),
  cs2: thumb("#1E2A33", "#0B1114", "#FF8A3D"),
  bg3: thumb("#2A2238", "#100D17", "#A78BFA"),
  satisfactory: thumb("#23301F", "#0B100A", "#FFB020"),
  desktop: thumb("#1A2229", "#0A0D10", "#7FB4FF"),
};

function thumbFor(game: string | null): string {
  switch (game) {
    case "ELDEN RING":
      return THUMBS.elden;
    case "Hades II":
      return THUMBS.hades;
    case "Counter-Strike 2":
      return THUMBS.cs2;
    case "Baldur's Gate 3":
      return THUMBS.bg3;
    case "Satisfactory":
      return THUMBS.satisfactory;
    default:
      return THUMBS.desktop;
  }
}

const now = Date.now();
const minutesAgo = (minutes: number) =>
  new Date(now - minutes * 60_000).toISOString();

let settings: Settings = {
  saveDir: "/home/user/Videos/Clipforge",
  bufferSeconds: 30,
  fps: 60,
  bitrateMbps: 20,
  encoder: "auto",
  audio: {
    desktopEnabled: true,
    desktopVolume: 0.8,
    micEnabled: true,
    micDevice: "alsa_input.pci-0000_00_1f.3.analog-stereo",
    micVolume: 1,
  },
  hotkeys: { ...DEFAULT_HOTKEYS },
  captureMonitor: {
    id: 1,
    name: "DP-1 · 2560×1440",
    restoreToken: "preview-token",
  },
  autoDetectGames: true,
  autoStartOnGame: true,
  saveOnGameExit: true,
  startBufferOnLaunch: false,
  minimizeToTray: true,
  launchAtLogin: false,
  notifyOnClip: true,
  clipSound: true,
  shareBaseUrl: "",
  shareToken: null,
  maxWidth: 2560,
};

/**
 * The preview normally opens mid-recording, which leaves the idle and error
 * states unreviewable. `?state=off|starting|saving|error` picks the recorder
 * state it opens in. This only exists in the browser preview backend; the
 * desktop app always answers from the real recorder.
 */
function previewRecorderState(): Partial<RecorderStatus> {
  switch (new URLSearchParams(window.location.search).get("state")) {
    case "off":
      return { state: "off", bufferedSeconds: 0, startedAt: null };
    case "starting":
      return { state: "starting", bufferedSeconds: 0, startedAt: null };
    case "saving":
      return { state: "saving", bufferedSeconds: 30 };
    case "error":
      return {
        state: "error",
        bufferedSeconds: 0,
        startedAt: null,
        error:
          "The screen capture portal denied access. Allow screen recording for Shmingus Clips, then try again.",
      };
    default:
      return {};
  }
}

let status: RecorderStatus = {
  state: "buffering",
  encoder: "NVENC H.264",
  hardwareEncoding: true,
  captureLabel: "DP-1 · 2560×1440",
  bufferedSeconds: 24,
  bufferTargetSeconds: 30,
  startedAt: new Date(now - 14 * 60_000).toISOString(),
  micActive: true,
  desktopAudioActive: true,
  error: null,
  ...previewRecorderState(),
};

let currentGame: GameInfo | null = {
  id: "steam:1245620",
  name: "ELDEN RING",
  source: "steam",
  pid: 4812,
};

let clips: Clip[] = [
  {
    id: "seed-clip-1",
    title: "Cliffside duel",
    game: "ELDEN RING",
    path: "/home/user/Videos/Clipforge/ELDEN RING/cliffside-duel-20261121-1634.mp4",
    thumbnail: THUMBS.elden,
    createdAt: minutesAgo(8),
    durationMs: 42_600,
    sizeBytes: 112_800_000,
    width: 2560,
    height: 1440,
    favorite: true,
  },
  {
    id: "seed-clip-2",
    title: "Chronos down",
    game: "Hades II",
    path: "/home/user/Videos/Clipforge/Hades II/chronos-down-20261121-1542.mp4",
    thumbnail: THUMBS.hades,
    createdAt: minutesAgo(54),
    durationMs: 31_200,
    sizeBytes: 74_400_000,
    width: 2560,
    height: 1440,
    favorite: false,
  },
  {
    id: "seed-clip-3",
    title: "Ace on Mirage",
    game: "Counter-Strike 2",
    path: "/home/user/Videos/Clipforge/Counter-Strike 2/ace-on-mirage-20261121-1425.mp4",
    thumbnail: THUMBS.cs2,
    createdAt: minutesAgo(3 * 60 + 12),
    durationMs: 24_800,
    sizeBytes: 58_900_000,
    width: 1920,
    height: 1080,
    favorite: false,
  },
  {
    id: "seed-clip-4",
    title: "That persuasion check",
    game: "Baldur's Gate 3",
    path: "/home/user/Videos/Clipforge/Baldur's Gate 3/that-persuasion-check-20261121-1308.mp4",
    thumbnail: THUMBS.bg3,
    createdAt: minutesAgo(6 * 60 + 40),
    durationMs: 58_400,
    sizeBytes: 151_300_000,
    width: 2560,
    height: 1440,
    favorite: true,
  },
  {
    id: "seed-clip-5",
    title: "First coal power",
    game: "Satisfactory",
    path: "/home/user/Videos/Clipforge/Satisfactory/first-coal-power-20261120-2147.mp4",
    thumbnail: THUMBS.satisfactory,
    createdAt: minutesAgo(19 * 60),
    durationMs: 74_900,
    sizeBytes: 196_200_000,
    width: 2560,
    height: 1440,
    favorite: false,
  },
  {
    id: "seed-clip-6",
    title: "Bug reproduced",
    game: null,
    path: "/home/user/Videos/Clipforge/bug-reproduced-20261120-1902.mp4",
    thumbnail: THUMBS.desktop,
    createdAt: minutesAgo(27 * 60),
    durationMs: 19_300,
    sizeBytes: 43_100_000,
    width: 2560,
    height: 1440,
    favorite: false,
  },
];

const environment: EnvironmentReport = {
  ok: false,
  sessionType: "wayland",
  desktop: "KDE",
  encoders: ["NVENC H.264 (hardware)", "x264 (software)"],
  // Mirrors `environment_report()` in src-tauri/src/diagnostics.rs: the backend
  // formats every check in plain language, so the preview seed does too.
  checks: [
    {
      id: "session",
      label: "Desktop session",
      ok: true,
      required: false,
      detail: "wayland (KDE)",
    },
    {
      id: "portal-screencast",
      label: "Screen recording",
      ok: true,
      required: true,
      detail: "available (version 6)",
    },
    {
      id: "portal-shortcuts",
      label: "Global hotkeys",
      ok: true,
      required: false,
      detail: "available (version 1)",
    },
    {
      id: "pipewire",
      label: "Audio and video server",
      ok: true,
      required: true,
      detail: "PipeWire is running",
    },
    {
      id: "ffmpeg",
      label: "Video tools",
      ok: false,
      required: true,
      detail: "ffmpeg not found in PATH",
    },
    {
      id: "ffprobe",
      label: "Video inspection",
      ok: true,
      required: true,
      detail: "available",
    },
    {
      id: "gst-pipewiresrc",
      label: "Screen capture module",
      ok: true,
      required: true,
      detail: "pipewiresrc present",
    },
    {
      id: "gst-mpegtsmux",
      label: "Clip container",
      ok: true,
      required: true,
      detail: "mpegtsmux present",
    },
    {
      id: "gst-splitmuxsink",
      label: "Clip splitter",
      ok: true,
      required: true,
      detail: "splitmuxsink present",
    },
    {
      id: "gst-nvh264enc",
      label: "Hardware video encoder",
      ok: true,
      required: false,
      detail: "nvh264enc present",
    },
    {
      id: "gst-x264enc",
      label: "Software video encoder",
      ok: true,
      required: true,
      detail: "x264enc present",
    },
  ],
};

const audioDevices: AudioDevice[] = [
  {
    id: "alsa_input.pci-0000_00_1f.3.analog-stereo",
    label: "Built-in Analog Stereo",
    isMonitor: false,
    isDefault: true,
  },
  {
    id: "alsa_input.usb-Blue_Microphones_Yeti_Nano-00.analog-stereo",
    label: "Yeti Nano",
    isMonitor: false,
    isDefault: false,
  },
  {
    id: "alsa_output.pci-0000_01_00.1.hdmi-stereo.monitor",
    label: "Monitor of HDMI Audio",
    isMonitor: true,
    isDefault: false,
  },
];

function slugify(value: string): string {
  return (
    value
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "") || "clip"
  );
}

function newId(): string {
  return `preview-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`;
}

/** Mirrors the backend: `<Game> clip - HH:MM` (24-hour local time). */
function generatedTitle(game: string | null): string {
  const time = new Intl.DateTimeFormat(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  }).format(new Date());
  return `${game ?? "Desktop"} clip - ${time}`;
}

function makeClip(overrides: {
  title: string;
  game: string | null;
  durationMs: number;
  thumbnail?: string;
}): Clip {
  const slug = slugify(overrides.title);
  const folder = overrides.game ?? "";
  const base = settings.saveDir.replace(/\/$/, "");
  const seconds = overrides.durationMs / 1000;
  return {
    id: newId(),
    title: overrides.title,
    game: overrides.game,
    path: `${base}/${folder ? `${folder}/` : ""}${slug}-${Date.now()}.mp4`,
    thumbnail: overrides.thumbnail ?? thumbFor(overrides.game),
    createdAt: new Date().toISOString(),
    durationMs: overrides.durationMs,
    sizeBytes: Math.round(seconds * (settings.bitrateMbps * 1_000_000) / 8),
    width: 2560,
    height: 1440,
    favorite: false,
  };
}

function findClip(id: string): Clip {
  const clip = clips.find((entry) => entry.id === id);
  if (!clip) throw new Error("Clip not found.");
  return clip;
}

const SHARE_UNAVAILABLE = "Sharing is available in the desktop app.";
const SIGN_IN_UNAVAILABLE = "Sign-in needs the desktop app.";

/**
 * The browser has no Authentik session. `?auth=signed-in` paints a sample
 * account so the signed-in sharing card can be reviewed; it is only a visual
 * state - signing in and out still say they need the desktop app.
 */
export function previewAuthSample(): boolean {
  return new URLSearchParams(window.location.search).get("auth") === "signed-in";
}

const PREVIEW_SIGNED_OUT: AuthStatus = {
  signedIn: false,
  name: null,
  email: null,
  sub: null,
  expiresAt: null,
};

const PREVIEW_ACCOUNT: AuthStatus = {
  signedIn: true,
  name: "Mara Vogel",
  email: "mara.vogel@example.com",
  sub: "mara.vogel",
  expiresAt: "2026-11-22T23:59:00Z",
};

function previewAuthStatus(): AuthStatus {
  return clone(previewAuthSample() ? PREVIEW_ACCOUNT : PREVIEW_SIGNED_OUT);
}

/** The settings section named by `?settings=`, for preview screenshots. */
export function previewSettingsParam(): string | null {
  return new URLSearchParams(window.location.search).get("settings");
}

/**
 * Preview-only sample so the share dialog can be reviewed in a plain browser.
 * `?share=<clip id>` opens it for that seed clip; nothing is uploaded.
 */
export function previewShareInfo(clip: Clip): ShareInfo {
  const id = `preview-${clip.id}`;
  return {
    id,
    url: `https://clips.shmingus.net/c/${id}`,
    embedUrl: `https://clips.shmingus.net/embed/${id}`,
    videoUrl: `https://clips.shmingus.net/media/${id}.mp4`,
    createdAt: new Date().toISOString(),
  };
}

/** The clip id named by `?share=`, used only by the browser preview. */
export function previewShareParam(): string | null {
  return new URLSearchParams(window.location.search).get("share");
}

export const previewBackend = {
  async getSettings(): Promise<Settings> {
    await wait(60);
    return clone(settings);
  },

  async saveSettings(next: Settings): Promise<Settings> {
    await wait(320);
    settings = clone(next);
    status = {
      ...status,
      micActive: next.audio.micEnabled,
      desktopAudioActive: next.audio.desktopEnabled,
      captureLabel: next.captureMonitor?.name ?? status.captureLabel,
    };
    return clone(settings);
  },

  async checkEnvironment(): Promise<EnvironmentReport> {
    await wait(420);
    return clone(environment);
  },

  async runSelftest(): Promise<SelfTestReport> {
    await wait(1400);
    return {
      ok: true,
      error: null,
      artifacts: ["/tmp/clipforge-selftest/selftest.mp4"],
      steps: [
        { name: "Create test source", ok: true, detail: "videotestsrc 1280×720 @ 60 fps" },
        { name: "Encode NVENC", ok: true, detail: "H.264 high profile, 20 Mbps" },
        { name: "Mux two audio tracks", ok: true, detail: "desktop tone + microphone tone" },
        { name: "Write MPEG-TS ring", ok: true, detail: "6 segments, 1 s each" },
        { name: "Concat + remux MP4", ok: true, detail: "5.9 s, 2 audio streams" },
        { name: "Generate thumbnail", ok: true, detail: "640 px JPEG at 1.0 s" },
      ],
    };
  },

  async getAudioDevices(): Promise<AudioDevice[]> {
    await wait(240);
    return clone(audioDevices);
  },

  async getRecorderStatus(): Promise<RecorderStatus> {
    await wait(40);
    return clone(status);
  },

  async startBuffer(): Promise<RecorderStatus> {
    await wait(600);
    status = {
      ...status,
      state: "buffering",
      bufferedSeconds: 0,
      startedAt: new Date().toISOString(),
      error: null,
      captureLabel: status.captureLabel ?? settings.captureMonitor?.name ?? "DP-1",
      bufferTargetSeconds: settings.bufferSeconds,
      micActive: settings.audio.micEnabled,
      desktopAudioActive: settings.audio.desktopEnabled,
    };
    return clone(status);
  },

  async stopBuffer(): Promise<RecorderStatus> {
    await wait(320);
    status = {
      ...status,
      state: "off",
      bufferedSeconds: 0,
      startedAt: null,
      error: null,
    };
    return clone(status);
  },

  async toggleMic(): Promise<Settings> {
    await wait(180);
    settings = {
      ...settings,
      audio: { ...settings.audio, micEnabled: !settings.audio.micEnabled },
    };
    status = { ...status, micActive: settings.audio.micEnabled };
    return clone(settings);
  },

  async chooseMonitor(): Promise<SavedMonitor> {
    await wait(700);
    const monitor: SavedMonitor = {
      id: 2,
      name: "HDMI-A-1 · 1920×1080",
      restoreToken: "preview-token-2",
    };
    settings = { ...settings, captureMonitor: monitor };
    status = { ...status, captureLabel: monitor.name };
    return clone(monitor);
  },

  async saveClip(title?: string): Promise<Clip> {
    await wait(820);
    const seconds = Math.max(
      1,
      Math.min(status.bufferedSeconds || status.bufferTargetSeconds, status.bufferTargetSeconds),
    );
    const game = currentGame?.name ?? null;
    const clip = makeClip({
      title: title?.trim() || generatedTitle(game),
      game,
      durationMs: seconds * 1000,
    });
    clips = [clip, ...clips];
    return clone(clip);
  },

  async getDetectedGame(): Promise<GameInfo | null> {
    await wait(40);
    return clone(currentGame);
  },

  async listClips(): Promise<Clip[]> {
    await wait(260);
    return clone(clips);
  },

  async deleteClip(id: string): Promise<void> {
    await wait(320);
    findClip(id);
    clips = clips.filter((clip) => clip.id !== id);
  },

  async renameClip(id: string, title: string): Promise<Clip> {
    await wait(260);
    const clip = findClip(id);
    const next: Clip = { ...clip, title: title.trim() || clip.title };
    clips = clips.map((entry) => (entry.id === id ? next : entry));
    return clone(next);
  },

  async setClipFavorite(id: string, favorite: boolean): Promise<Clip> {
    await wait(140);
    const clip = findClip(id);
    const next: Clip = { ...clip, favorite };
    clips = clips.map((entry) => (entry.id === id ? next : entry));
    return clone(next);
  },

  async trimClip(id: string, startMs: number, endMs: number): Promise<Clip> {
    await wait(900);
    const clip = findClip(id);
    const durationMs = Math.min(clip.durationMs, Math.max(100, endMs - startMs));
    const trimmed = makeClip({
      title: `${clip.title} (trim)`,
      game: clip.game,
      durationMs,
      thumbnail: clip.thumbnail ?? undefined,
    });
    trimmed.favorite = false;
    clips = [trimmed, ...clips];
    return clone(trimmed);
  },

  async exportClip(id: string): Promise<string> {
    await wait(520);
    const clip = findClip(id);
    return `${settings.saveDir.replace(/\/$/, "")}/Exports/${slugify(clip.title)}.mp4`;
  },

  async revealExport(path: string): Promise<void> {
    await wait(120);
    if (!path) throw new Error("No exported file to open.");
  },

  async openClipsFolder(): Promise<void> {
    await wait(120);
  },

  async revealClip(id: string): Promise<void> {
    await wait(120);
    findClip(id);
  },

  // The browser has no share server behind it. Every sharing call fails the
  // same way, so the interface can say why instead of pretending to upload.
  async shareClip(_id: string): Promise<ShareInfo> {
    await wait(160);
    throw new Error(SHARE_UNAVAILABLE);
  },

  async getShare(_id: string): Promise<ShareInfo | null> {
    await wait(120);
    throw new Error(SHARE_UNAVAILABLE);
  },

  async listShares(): Promise<Record<string, ShareInfo>> {
    await wait(120);
    throw new Error(SHARE_UNAVAILABLE);
  },

  async deleteShare(_id: string): Promise<void> {
    await wait(120);
    throw new Error(SHARE_UNAVAILABLE);
  },

  async testShareServer(): Promise<ShareServerStatus> {
    await wait(320);
    throw new Error(SHARE_UNAVAILABLE);
  },

  async authStatus(): Promise<AuthStatus> {
    await wait(80);
    return previewAuthStatus();
  },

  async authLogin(): Promise<AuthStatus> {
    await wait(120);
    throw new Error(SIGN_IN_UNAVAILABLE);
  },

  async authLogout(): Promise<void> {
    await wait(120);
    throw new Error(SIGN_IN_UNAVAILABLE);
  },
};
