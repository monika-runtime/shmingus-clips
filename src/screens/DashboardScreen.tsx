import { Clapperboard } from "lucide-react";

import { OnboardingCard } from "../components/OnboardingCard";
import { RecorderCard } from "../components/RecorderCard";
import { RecentClips } from "../components/RecentClips";
import { SetupCard } from "../components/SetupCard";
import { Alert } from "../components/ui/Alert";
import { Badge } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { PageHeader } from "../components/ui/PageHeader";
import type { RecorderState } from "../lib/types";
import { useApp } from "../state/store";

const STATUS_PILL: Record<
  RecorderState,
  { label: string; tone: "neutral" | "accent" | "danger" }
> = {
  off: { label: "Idle", tone: "neutral" },
  starting: { label: "Recording", tone: "accent" },
  buffering: { label: "Recording", tone: "accent" },
  saving: { label: "Recording", tone: "accent" },
  error: { label: "Error", tone: "danger" },
};

export function DashboardScreen() {
  const { clipsError, refreshClips, settingsError, status } = useApp();

  const pill = STATUS_PILL[status.state];
  const recording = status.state !== "off" && status.state !== "error";

  return (
    <div className="page">
      <PageHeader
        eyebrow="Dashboard"
        title="Replay buffer"
        description="Save the last few seconds of play whenever something worth keeping happens."
        actions={
          <Badge
            tone={pill.tone}
            dot
            pulse={recording}
            className="status-pill"
          >
            {pill.label}
          </Badge>
        }
      />

      <RecorderCard />

      <OnboardingCard />

      {settingsError && (
        <Alert tone="warn" title="Settings unavailable">
          Using the built-in defaults because the settings backend did not
          answer: {settingsError}
        </Alert>
      )}

      {clipsError && (
        <Alert
          tone="warn"
          title="Library unavailable"
          action={
            <Button size="sm" variant="secondary" onClick={() => void refreshClips()}>
              Retry
            </Button>
          }
        >
          {clipsError}
        </Alert>
      )}

      <RecentClips />

      <SetupCard />

      <p className="page-footnote">
        <Clapperboard size={14} aria-hidden="true" /> Clips are saved on this PC.
        Sharing is optional and uploads only the clips you choose.
      </p>
    </div>
  );
}
