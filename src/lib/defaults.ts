import type { RecorderStatus, Settings } from "./types";

export const APP_VERSION = "0.3.0";

export const DEFAULT_HOTKEYS = {
  saveClip: "ALT+c",
  toggleBuffer: "CTRL+ALT+b",
  toggleMic: "CTRL+ALT+m",
};

/** Mirrors `Settings::default()` in `src-tauri/src/model.rs`. */
export const DEFAULT_SETTINGS: Settings = {
  saveDir: "",
  bufferSeconds: 30,
  fps: 60,
  bitrateMbps: 20,
  encoder: "auto",
  audio: {
    desktopEnabled: true,
    desktopVolume: 1,
    micEnabled: true,
    micDevice: null,
    micVolume: 1,
  },
  hotkeys: DEFAULT_HOTKEYS,
  captureMonitor: null,
  autoDetectGames: true,
  autoStartOnGame: true,
  saveOnGameExit: true,
  startBufferOnLaunch: false,
  minimizeToTray: true,
  launchAtLogin: false,
  notifyOnClip: true,
  clipSound: true,
  shareBaseUrl: "https://clips.shmingus.net",
  shareToken: null,
  maxWidth: 1920,
};

/** Mirrors `RecorderStatus::default()`. */
export const DEFAULT_RECORDER_STATUS: RecorderStatus = {
  state: "off",
  encoder: null,
  hardwareEncoding: false,
  captureLabel: null,
  bufferedSeconds: 0,
  bufferTargetSeconds: 30,
  startedAt: null,
  micActive: false,
  desktopAudioActive: false,
  error: null,
};
