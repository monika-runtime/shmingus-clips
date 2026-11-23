import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

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
  Toast,
} from "./types";

export const api = {
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) =>
    invoke<Settings>("save_settings", { settings }),

  /** Base URL of the loopback clip server used by <video>. */
  getMediaBase: () => invoke<string>("get_media_base"),

  checkEnvironment: () => invoke<EnvironmentReport>("check_environment"),
  runSelftest: () => invoke<SelfTestReport>("run_selftest"),
  getAudioDevices: () => invoke<AudioDevice[]>("get_audio_devices"),

  getRecorderStatus: () => invoke<RecorderStatus>("get_recorder_status"),
  startBuffer: () => invoke<RecorderStatus>("start_buffer"),
  stopBuffer: () => invoke<RecorderStatus>("stop_buffer"),
  toggleMic: () => invoke<Settings>("toggle_mic"),
  chooseMonitor: () => invoke<SavedMonitor>("choose_monitor"),
  saveClip: (title?: string) => invoke<Clip>("save_clip", { title: title ?? null }),

  getDetectedGame: () => invoke<GameInfo | null>("get_detected_game"),

  listClips: () => invoke<Clip[]>("list_clips"),
  deleteClip: (id: string) => invoke<void>("delete_clip", { id }),
  renameClip: (id: string, title: string) =>
    invoke<Clip>("rename_clip", { id, title }),
  setClipFavorite: (id: string, favorite: boolean) =>
    invoke<Clip>("set_clip_favorite", { id, favorite }),
  trimClip: (id: string, startMs: number, endMs: number) =>
    invoke<Clip>("trim_clip", { id, startMs, endMs }),
  exportClip: (id: string, destDir?: string) =>
    invoke<string>("export_clip", { id, destDir: destDir ?? null }),
  /** Open the folder that holds a file the library just exported. */
  revealExport: (path: string) => invoke<void>("reveal_export", { path }),
  openClipsFolder: () => invoke<void>("open_clips_folder"),
  revealClip: (id: string) => invoke<void>("reveal_clip", { id }),

  /** Upload a clip to the configured share server and return its links. */
  shareClip: (id: string) => invoke<ShareInfo>("share_clip", { id }),
  /** The share for one clip, or null when it is not on the server. */
  getShare: (id: string) => invoke<ShareInfo | null>("get_share", { id }),
  /** Every share on the server, keyed by clip id. */
  listShares: () => invoke<Record<string, ShareInfo>>("list_shares"),
  deleteShare: (id: string) => invoke<void>("delete_share", { id }),
  testShareServer: () => invoke<ShareServerStatus>("test_share_server"),

  /** The Authentik account signed in on this machine, if any. */
  authStatus: () => invoke<AuthStatus>("auth_status"),
  /**
   * Open the system browser for Authentik sign-in (PKCE) and wait for the
   * callback. Can take a few minutes while the user finishes in the browser.
   */
  authLogin: () => invoke<AuthStatus>("auth_login"),
  authLogout: () => invoke<void>("auth_logout"),
};

/** Local file path -> URL playable in <video>/<img> through the asset protocol. */
export function assetUrl(path: string): string {
  return convertFileSrc(path);
}

export function onRecorderStatus(cb: (s: RecorderStatus) => void): Promise<UnlistenFn> {
  return listen<RecorderStatus>("recorder:status", (e) => cb(e.payload));
}

export function onClipSaved(cb: (clip: Clip) => void): Promise<UnlistenFn> {
  return listen<Clip>("clip:saved", (e) => cb(e.payload));
}

export function onGameDetected(
  cb: (game: GameInfo | null) => void,
): Promise<UnlistenFn> {
  return listen<GameInfo | null>("game:detected", (e) => cb(e.payload));
}

export function onLibraryChanged(cb: () => void): Promise<UnlistenFn> {
  return listen("library:changed", () => cb());
}

export function onToast(cb: (toast: Toast) => void): Promise<UnlistenFn> {
  return listen<Toast>("toast", (e) => cb(e.payload));
}
