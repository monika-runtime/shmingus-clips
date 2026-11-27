import { Download, X } from "lucide-react";
import { useState } from "react";

import { APP_VERSION } from "../lib/defaults";
import { errorMessage } from "../lib/errors";
import { downloadAndInstall } from "../lib/updates";
import { useApp } from "../state/store";
import { Button, IconButton } from "./ui/Button";

/**
 * A release found at launch. Unlike a toast this stays until the person acts,
 * so an update cannot slip by while a game has focus.
 */
export function UpdateBanner() {
  const { update, dismissUpdate, requiredClientVersion, openSettings } = useApp();
  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  const required = requiredClientVersion;
  if (!update && !required) return null;
  const updateRequired = Boolean(required);

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
    : (error ??
      (updateRequired
        ? "This build is older than your share server supports. Updating keeps sharing and embeds working."
        : "Install now, or dismiss this and keep working."));

  return (
    <div className="banner banner--update" role="status">
      <span className="banner__icon" aria-hidden="true">
        <Download size={18} />
      </span>
      <div className="banner__text">
        <p className="banner__title">
          {updateRequired
            ? `Update required: your server needs ${required}`
            : `Shmingus Clips ${update?.version} is available`}
        </p>
        <p className="banner__message">
          {updateRequired && `You have v${APP_VERSION}. `}
          {message}
        </p>
      </div>
      {update ? (
        <Button
          size="sm"
          variant="primary"
          disabled={installing}
          onClick={() => void install()}
        >
          {installing ? "Installing…" : "Install and restart"}
        </Button>
      ) : (
        <Button
          size="sm"
          variant="primary"
          onClick={() => openSettings("updates")}
        >
          Open updates
        </Button>
      )}
      {update && !updateRequired && (
        <IconButton
          label="Dismiss update notice"
          disabled={installing}
          onClick={dismissUpdate}
        >
          <X size={16} />
        </IconButton>
      )}
    </div>
  );
}
