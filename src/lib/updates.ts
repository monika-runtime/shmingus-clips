import { isDesktop } from "./bridge";

/** A release newer than the running one, as GitHub advertises it. */
export interface UpdateInfo {
  version: string;
  notes: string | null;
  date: string | null;
}

/**
 * Ask the release feed for a newer version. Returns null when the app is
 * current, when running in the browser preview, or when this build has no
 * updater configured.
 */
export async function checkForUpdate(): Promise<UpdateInfo | null> {
  if (!isDesktop()) return null;
  const { check } = await import("@tauri-apps/plugin-updater");
  const update = await check();
  if (!update) return null;
  return {
    version: update.version,
    notes: update.body ?? null,
    date: update.date ?? null,
  };
}

/**
 * Download and install the pending update, then restart into it. `onProgress`
 * receives a percentage when the server reports the total size, else null.
 */
export async function downloadAndInstall(
  onProgress: (percent: number | null) => void,
): Promise<void> {
  const { check } = await import("@tauri-apps/plugin-updater");
  const update = await check();
  if (!update) throw new Error("No update is available.");

  let total = 0;
  let received = 0;
  await update.downloadAndInstall((event) => {
    if (event.event === "Started") {
      total = event.data.contentLength ?? 0;
      received = 0;
      onProgress(total > 0 ? 0 : null);
    } else if (event.event === "Progress") {
      received += event.data.chunkLength;
      onProgress(total > 0 ? Math.min(99, Math.round((received / total) * 100)) : null);
    } else {
      onProgress(100);
    }
  });

  const { relaunch } = await import("@tauri-apps/plugin-process");
  await relaunch();
}
