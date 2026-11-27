import type { UnlistenFn } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";

import { api, assetUrl, onMicLevel } from "./api";
import { previewBackend } from "./preview";
import type { EditPlan, MicLevel } from "./types";

/**
 * The frontend talks to the backend through this bridge. Inside Tauri every
 * call goes to `api.ts` unchanged; in a plain browser (`pnpm dev` without the
 * app shell) the preview backend answers with seed data so the interface can
 * be reviewed. Callers still have to catch rejections in both modes.
 */

export function isDesktop(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function call<T>(
  desktop: () => Promise<T>,
  preview: () => Promise<T>,
): Promise<T> {
  return isDesktop() ? desktop() : preview();
}

export const backend = {
  getSettings: () => call(api.getSettings, previewBackend.getSettings),
  saveSettings: (settings: Parameters<typeof api.saveSettings>[0]) =>
    call(
      () => api.saveSettings(settings),
      () => previewBackend.saveSettings(settings),
    ),

  getMediaBase: () => call(api.getMediaBase, async () => ""),

  checkEnvironment: () =>
    call(api.checkEnvironment, previewBackend.checkEnvironment),
  runSelftest: () => call(api.runSelftest, previewBackend.runSelftest),
  bugReport: () => call(api.bugReport, previewBackend.bugReport),
  getAudioDevices: () =>
    call(api.getAudioDevices, previewBackend.getAudioDevices),
  renderEdit: (plan: EditPlan) =>
    call(
      () => api.renderEdit(plan),
      () => previewBackend.renderEdit(plan),
    ),
  listServerShares: () =>
    call(api.listServerShares, previewBackend.listServerShares),
  deleteServerShare: (id: string) =>
    call(
      () => api.deleteServerShare(id),
      () => previewBackend.deleteServerShare(id),
    ),
  micLevel: () => call(api.micLevel, previewBackend.micLevel),
  startMicMonitor: () => call(api.startMicMonitor, previewBackend.startMicMonitor),
  stopMicMonitor: () => call(api.stopMicMonitor, previewBackend.stopMicMonitor),
  onMicLevel: (cb: (level: MicLevel) => void): Promise<UnlistenFn> =>
    isDesktop() ? onMicLevel(cb) : previewBackend.onMicLevel(cb),

  getRecorderStatus: () =>
    call(api.getRecorderStatus, previewBackend.getRecorderStatus),
  startBuffer: () => call(api.startBuffer, previewBackend.startBuffer),
  stopBuffer: () => call(api.stopBuffer, previewBackend.stopBuffer),
  toggleMic: () => call(api.toggleMic, previewBackend.toggleMic),
  chooseMonitor: () => call(api.chooseMonitor, previewBackend.chooseMonitor),
  saveClip: (title?: string) =>
    call(
      () => api.saveClip(title),
      () => previewBackend.saveClip(title),
    ),

  getDetectedGame: () =>
    call(api.getDetectedGame, previewBackend.getDetectedGame),

  listClips: () => call(api.listClips, previewBackend.listClips),
  deleteClip: (id: string) =>
    call(
      () => api.deleteClip(id),
      () => previewBackend.deleteClip(id),
    ),
  renameClip: (id: string, title: string) =>
    call(
      () => api.renameClip(id, title),
      () => previewBackend.renameClip(id, title),
    ),
  setClipFavorite: (id: string, favorite: boolean) =>
    call(
      () => api.setClipFavorite(id, favorite),
      () => previewBackend.setClipFavorite(id, favorite),
    ),
  trimClip: (id: string, startMs: number, endMs: number) =>
    call(
      () => api.trimClip(id, startMs, endMs),
      () => previewBackend.trimClip(id, startMs, endMs),
    ),
  exportClip: (id: string) =>
    call(
      () => api.exportClip(id),
      () => previewBackend.exportClip(id),
    ),
  revealExport: (path: string) =>
    call(
      () => api.revealExport(path),
      () => previewBackend.revealExport(path),
    ),
  openClipsFolder: () =>
    call(api.openClipsFolder, previewBackend.openClipsFolder),
  revealClip: (id: string) =>
    call(
      () => api.revealClip(id),
      () => previewBackend.revealClip(id),
    ),

  shareClip: (id: string) =>
    call(
      () => api.shareClip(id),
      () => previewBackend.shareClip(id),
    ),
  getShare: (id: string) =>
    call(
      () => api.getShare(id),
      () => previewBackend.getShare(id),
    ),
  listShares: () => call(api.listShares, previewBackend.listShares),
  deleteShare: (id: string) =>
    call(
      () => api.deleteShare(id),
      () => previewBackend.deleteShare(id),
    ),
  testShareServer: () =>
    call(api.testShareServer, previewBackend.testShareServer),
  serverRequirement: () =>
    call(api.serverRequirement, previewBackend.serverRequirement),

  authStatus: () => call(api.authStatus, previewBackend.authStatus),
  authLogin: () => call(api.authLogin, previewBackend.authLogin),
  authLogout: () => call(api.authLogout, previewBackend.authLogout),
};

/** Local file path -> asset URL. Seed thumbnails are data URIs already. */
export function mediaUrl(path: string): string {
  if (/^(data:|https?:|blob:)/.test(path)) return path;
  return isDesktop() ? assetUrl(path) : path;
}

/**
 * Playable URL for a clip. WebKitGTK plays media through GStreamer, which does
 * not understand Tauri's asset protocol, so the backend serves clips over a
 * loopback HTTP server (with range support for seeking).
 */
export function videoUrl(clip: { id: string }, mediaBase: string): string {
  if (!mediaBase) return "";
  return `${mediaBase}/media/${encodeURIComponent(clip.id)}`;
}

/** Native folder picker; the browser preview answers with a plausible folder. */
export async function chooseDirectory(
  defaultPath?: string,
): Promise<string | null> {
  if (!isDesktop()) return "/home/user/Videos/Shmingus Clips";
  const picked = await open({
    directory: true,
    multiple: false,
    title: "Choose where clips are saved",
    defaultPath,
  });
  return typeof picked === "string" ? picked : null;
}
