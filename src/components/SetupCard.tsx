import { AlertTriangle, Check, RefreshCw, Stethoscope } from "lucide-react";

import { useApp } from "../state/store";
import { Alert } from "./ui/Alert";
import { Button } from "./ui/Button";
import { Spinner } from "./ui/Spinner";

export function SetupCard() {
  const {
    environment,
    environmentError,
    environmentBusy,
    refreshEnvironment,
    openSettings,
  } = useApp();

  if (environmentError) {
    return (
      <section className="card setup-card" aria-labelledby="setup-title">
        <div className="setup-card__head">
          <span className="setup-card__icon" aria-hidden="true">
            <AlertTriangle size={18} />
          </span>
          <div>
            <p className="micro-label">Environment</p>
            <h2 className="setup-card__title" id="setup-title">
              Could not check this system
            </h2>
          </div>
        </div>
        <Alert tone="warn">{environmentError}</Alert>
        <div className="setup-card__actions">
          <Button
            variant="secondary"
            size="sm"
            icon={environmentBusy ? <Spinner /> : <RefreshCw size={15} />}
            onClick={() => void refreshEnvironment()}
            disabled={environmentBusy}
          >
            Try again
          </Button>
        </div>
      </section>
    );
  }

  if (!environment || environment.ok) return null;

  const failing = environment.checks.filter(
    (check) => !check.ok && check.required,
  );
  if (failing.length === 0) return null;

  const warnings = environment.checks.filter(
    (check) => !check.ok && !check.required,
  );

  return (
    <section className="card setup-card" aria-labelledby="setup-title">
      <div className="setup-card__head">
        <span className="setup-card__icon" aria-hidden="true">
          <AlertTriangle size={18} />
        </span>
        <div className="setup-card__heading">
          <p className="micro-label">Environment</p>
          <h2 className="setup-card__title" id="setup-title">
            Finish setting up capture
          </h2>
          <p className="setup-card__intro">
            These required pieces are missing. The rest of Shmingus Clips keeps
            working while you fix them.
          </p>
        </div>
        <Button
          variant="ghost"
          size="sm"
          icon={<Stethoscope size={15} />}
          onClick={() => openSettings("diagnostics")}
        >
          Diagnostics
        </Button>
      </div>

      <ul className="check-list">
        {failing.map((check) => (
          <li key={check.id} className="check-list__item">
            <span className="check-list__status is-bad" aria-hidden="true">
              <AlertTriangle size={15} />
            </span>
            <div className="check-list__body">
              <p className="check-list__label">{check.label}</p>
              <p className="check-list__detail">{check.detail}</p>
            </div>
            <span className="check-list__tag">Required</span>
          </li>
        ))}
      </ul>

      {warnings.length > 0 && (
        <p className="setup-card__warnings">
          <Check size={14} aria-hidden="true" />{" "}
          {warnings.length} optional{" "}
          {warnings.length === 1 ? "check is" : "checks are"} unavailable:{" "}
          {warnings.map((check) => check.label).join(", ")}.
        </p>
      )}

      <div className="setup-card__actions">
        <Button
          variant="secondary"
          size="sm"
          icon={environmentBusy ? <Spinner /> : <RefreshCw size={15} />}
          onClick={() => void refreshEnvironment()}
          disabled={environmentBusy}
        >
          Check again
        </Button>
      </div>
    </section>
  );
}
