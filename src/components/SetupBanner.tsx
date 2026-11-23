import { AlertTriangle, X } from "lucide-react";

import { useApp } from "../state/store";
import { Button } from "./ui/Button";
import { IconButton } from "./ui/Button";

export function SetupBanner() {
  const {
    environment,
    setupDismissed,
    dismissSetup,
    openSettings,
  } = useApp();

  if (!environment || environment.ok || setupDismissed) return null;
  const failing = environment.checks.filter(
    (check) => !check.ok && check.required,
  );
  if (failing.length === 0) return null;

  return (
    <div className="banner" role="status">
      <span className="banner__icon" aria-hidden="true">
        <AlertTriangle size={18} />
      </span>
      <div className="banner__text">
        <p className="banner__title">Setup needed</p>
        <p className="banner__message">
          {failing.length} required system{" "}
          {failing.length === 1 ? "check has" : "checks have"} failed - recording
          stays unavailable until {failing.length === 1 ? "it is" : "they are"}{" "}
          fixed.
        </p>
      </div>
      <Button
        size="sm"
        variant="secondary"
        onClick={() => openSettings("diagnostics")}
      >
        Open diagnostics
      </Button>
      <IconButton label="Dismiss setup notice" onClick={dismissSetup}>
        <X size={16} />
      </IconButton>
    </div>
  );
}
