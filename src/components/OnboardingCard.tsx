import { Clapperboard, Play, X } from "lucide-react";

import { DEFAULT_HOTKEYS } from "../lib/defaults";
import { formatAccelerator, pluralize } from "../lib/format";
import { useApp } from "../state/store";
import { Button, IconButton } from "./ui/Button";
import { Spinner } from "./ui/Spinner";

/**
 * One-time, dismissible explainer. It says what happens (a clip of the last
 * moments ends up in Videos), not how the replay buffer works, and it stays out
 * of the way: it is inline, never a modal, and stays dismissed.
 */
export function OnboardingCard() {
  const {
    settings,
    onboardingDismissed,
    dismissOnboarding,
    status,
    busy,
    startBuffer,
  } = useApp();

  if (onboardingDismissed) return null;

  const seconds = settings?.bufferSeconds ?? 30;
  const hotkey = formatAccelerator(
    settings?.hotkeys.saveClip ?? DEFAULT_HOTKEYS.saveClip,
  ).join("+");
  const auto = settings?.autoStartOnGame === true;
  const canStart = status.state === "off" || status.state === "error";

  return (
    <section className="card onboarding" aria-labelledby="onboarding-title">
      <span className="onboarding__icon" aria-hidden="true">
        <Clapperboard size={20} />
      </span>
      <div className="onboarding__body">
        <p className="micro-label">Getting started</p>
        <h2 className="onboarding__title" id="onboarding-title">
          Save what just happened
        </h2>
        <p className="onboarding__text">
          {auto ? (
            <>
              Shmingus Clips starts when a game launches. Press {hotkey} when
              something happens - the clip lands in your Videos folder.
            </>
          ) : (
            <>
              Shmingus Clips keeps your last {pluralize(seconds, "second")} ready.
              Press Start, then {hotkey} when something happens - the clip lands
              in your Videos folder.
            </>
          )}
        </p>
        {canStart && !auto && (
          <Button
            size="sm"
            variant="primary"
            icon={busy.start ? <Spinner /> : <Play size={15} />}
            disabled={busy.start}
            onClick={() => void startBuffer()}
          >
            {busy.start ? "Starting…" : "Start replay buffer"}
          </Button>
        )}
      </div>
      <IconButton label="Dismiss getting started card" onClick={dismissOnboarding}>
        <X size={16} />
      </IconButton>
    </section>
  );
}
