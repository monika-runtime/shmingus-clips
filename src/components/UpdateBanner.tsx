import { Download, X } from "lucide-react";
import { useState } from "react";

import { errorMessage } from "../lib/errors";
import { downloadAndInstall } from "../lib/updates";
import { useApp } from "../state/store";
import { Button, IconButton } from "./ui/Button";

/**
 * A release found at launch. Unlike a toast this stays until the person acts,
 * so an update cannot slip by while a game has focus.
 */
export function UpdateBanner() {
  const { update, dismissUpdate } = useApp();
  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  if (!update) return null;

  async function install() {
    setInstalling(true);
    setError(null);
    try {
      await downloadAndInstall(setProgress);
      // The app restarts into the new version; this window goes away with it.
    } catch (err) {
      setError(errorMessage(err));
      setInstalling(false);
      setProgress(null);
    }
  }

  const message = installing
    ? progress === null
      ? "Downloading and installing…"
      : `Downloading and installing… ${progress}%`
    : (error ?? "Install now, or dismiss this and keep working.");

  return (
    <div className="banner banner--update" role="status">
      <span className="banner__icon" aria-hidden="true">
        <Download size={18} />
      </span>
      <div className="banner__text">
        <p className="banner__title">
          Shmingus Clips {update.version} is available
        </p>
        <p className="banner__message">{message}</p>
      </div>
      <Button
        size="sm"
        variant="primary"
        disabled={installing}
        onClick={() => void install()}
      >
        {installing ? "Installing…" : "Install and restart"}
      </Button>
      <IconButton
        label="Dismiss update notice"
        disabled={installing}
        onClick={dismissUpdate}
      >
        <X size={16} />
      </IconButton>
    </div>
  );
}
