import { backend } from "../lib/bridge";
import { errorMessage } from "../lib/errors";
import type { Clip } from "../lib/types";
import { useApp } from "./store";

/** Clip mutations shared by the library grid and the player modal. */
export function useClipActions() {
  const { patchClip, removeClip, pushToast } = useApp();

  async function toggleFavorite(clip: Clip): Promise<Clip | null> {
    try {
      const updated = await backend.setClipFavorite(clip.id, !clip.favorite);
      patchClip(updated);
      return updated;
    } catch (error) {
      pushToast("error", errorMessage(error));
      return null;
    }
  }

  async function rename(clip: Clip, title: string): Promise<Clip | null> {
    const cleaned = title.trim();
    if (!cleaned) {
      pushToast("error", "Give the clip a name first.");
      return null;
    }
    try {
      const updated = await backend.renameClip(clip.id, cleaned);
      patchClip(updated);
      pushToast("success", `Renamed to “${updated.title}”.`);
      return updated;
    } catch (error) {
      pushToast("error", errorMessage(error));
      return null;
    }
  }

  async function remove(clip: Clip): Promise<boolean> {
    try {
      await backend.deleteClip(clip.id);
      removeClip(clip.id);
      pushToast("success", `Deleted “${clip.title}”.`);
      return true;
    } catch (error) {
      pushToast("error", errorMessage(error));
      return false;
    }
  }

  /** Open the folder holding an export. The backend refuses paths outside the library. */
  async function showExportFolder(path: string): Promise<void> {
    try {
      await backend.revealExport(path);
    } catch (error) {
      pushToast("error", errorMessage(error));
    }
  }

  /**
   * Copy the clip to the Exports folder and return the absolute path, so the
   * caller can keep showing it. The toast names the destination and offers the
   * one click that matters next: opening it.
   */
  async function exportClip(clip: Clip): Promise<string | null> {
    try {
      const path = await backend.exportClip(clip.id);
      pushToast("success", `Exported to ${path}`, {
        label: "Show folder",
        run: () => void showExportFolder(path),
      });
      return path;
    } catch (error) {
      pushToast("error", errorMessage(error));
      return null;
    }
  }

  async function reveal(clip: Clip): Promise<void> {
    try {
      await backend.revealClip(clip.id);
    } catch (error) {
      pushToast("error", errorMessage(error));
    }
  }

  return {
    toggleFavorite,
    rename,
    remove,
    exportClip,
    showExportFolder,
    reveal,
  };
}
