import {
  Clapperboard,
  Cpu,
  FolderOpen,
  Gamepad2,
  Monitor,
  Play,
  Settings2,
  Square,
} from "lucide-react";
import { useState, type MouseEvent as ReactMouseEvent } from "react";

import { backend } from "../lib/bridge";
import { DEFAULT_HOTKEYS } from "../lib/defaults";
import { errorMessage } from "../lib/errors";
import { clamp, formatAccelerator, formatResolution } from "../lib/format";
import { isNativeContextTarget, useContextMenu } from "../state/contextMenu";
import { useApp } from "../state/store";
import { Badge } from "./ui/Badge";
import { Button } from "./ui/Button";
import { Hotkey } from "./ui/Hotkey";
import { Spinner } from "./ui/Spinner";
import { Switch } from "./ui/Switch";

const ENCODER_LABEL = {
  auto: "Auto encoder",
  nvenc: "NVENC encoder",
  x264: "x264 encoder",
};

/**
 * The recorder, reduced to what the person acts on: one primary action, the
 * buffer as a single left-to-right timeline, and the audio/encoder choices as
 * one quiet line underneath. Capture state lives in the dashboard header only.
 */
export function RecorderCard() {
  const {
    status,
    statusError,
    settings,
    game,
    busy,
    saveClipNow,
    startBuffer,
    stopBuffer,
    toggleMic,
    applySettings,
    pushToast,
    openSettings,
  } = useApp();
  const { openContextMenu } = useContextMenu();
  const [audioBusy, setAudioBusy] = useState(false);
  const [audioError, setAudioError] = useState<string | null>(null);

  const target = status.bufferTargetSeconds || settings?.bufferSeconds || 30;
  const buffered = Math.min(status.bufferedSeconds, target);
  const buffering = status.state === "buffering";
  const starting = status.state === "starting";
  const saving = status.state === "saving" || busy.save;
  const errored = status.state === "error";
  const idle = status.state === "off";
  const live = buffering || status.state === "saving";

  const encoder = settings ? ENCODER_LABEL[settings.encoder] : "Auto encoder";
  const width = settings?.maxWidth || 1920;
  const resolution = formatResolution(width, Math.round((width * 9) / 16));
  const micOn = settings?.audio.micEnabled ?? status.micActive;
  const desktopOn =
    settings?.audio.desktopEnabled ?? status.desktopAudioActive;
  const hotkey = settings?.hotkeys.saveClip ?? DEFAULT_HOTKEYS.saveClip;
  const fillPercent =
    target > 0 ? clamp((buffered / target) * 100, 0, 100) : 0;
  const autoHint = idle && settings?.autoStartOnGame === true;

  function openMenu(event: ReactMouseEvent<HTMLElement>) {
    if (isNativeContextTarget(event.target)) return;
    event.preventDefault();
    event.stopPropagation();
    const running = buffering || starting;
    openContextMenu(event, [
      running
        ? {
            label: "Stop replay buffer",
            icon: Square,
            disabled: busy.stop || starting,
            onSelect: () => void stopBuffer(),
          }
        : {
            label: "Start replay buffer",
            icon: Play,
            disabled: busy.start || saving,
            onSelect: () => void startBuffer(),
          },
      {
        label: "Save clip",
        icon: Clapperboard,
        shortcut: formatAccelerator(hotkey).join(" + "),
        disabled: !(buffering || saving) || busy.save,
        onSelect: () => void saveClipNow(),
      },
      { separator: true },
      {
        label: "Settings → Capture",
        icon: Settings2,
        onSelect: () => openSettings("capture"),
      },
      {
        label: "Open clips folder",
        icon: FolderOpen,
        onSelect: () => {
          void backend.openClipsFolder().catch((error: unknown) => {
            pushToast("error", errorMessage(error));
          });
        },
      },
    ]);
  }

  async function setDesktopAudio(next: boolean) {
    if (!settings) return;
    setAudioBusy(true);
    setAudioError(null);
    try {
      await applySettings({
        ...settings,
        audio: { ...settings.audio, desktopEnabled: next },
      });
    } catch (error) {
      const message = errorMessage(error);
      setAudioError(message);
      pushToast("error", message);
    } finally {
      setAudioBusy(false);
    }
  }

  return (
    <section
      className={live ? "card recorder is-live" : "card recorder"}
      aria-labelledby="recorder-title"
      onContextMenu={openMenu}
    >
      <h2 className="sr-only" id="recorder-title">
        Replay buffer
      </h2>

      <div className="recorder__main">
        <div className="recorder__lead">
          {idle && (
            <>
              <Button
                variant="primary"
                size="lg"
                icon={busy.start ? <Spinner size={18} /> : <Play size={18} />}
                onClick={() => void startBuffer()}
                disabled={busy.start || saving}
              >
                {busy.start
                  ? "Starting…"
                  : "Start replay buffer"}
              </Button>
              <p className="recorder__hint">
                Then press <Hotkey accelerator={hotkey} /> when something
                happens.
              </p>
            </>
          )}

          {starting && (
            <Button
              variant="primary"
              size="lg"
              icon={<Spinner size={18} />}
              disabled
            >
              Starting…
            </Button>
          )}

          {(buffering || status.state === "saving") && (
            <div className="recorder__actions">
              <Button
                variant="primary"
                size="lg"
                icon={saving ? <Spinner size={18} /> : <Clapperboard size={18} />}
                trailing={<Hotkey accelerator={hotkey} className="btn__hotkey" />}
                disabled={!buffering || busy.save}
                onClick={() => void saveClipNow()}
              >
                {saving ? "Saving…" : "Save clip"}
              </Button>
              <Button
                variant="secondary"
                size="lg"
                icon={busy.stop ? <Spinner /> : <Square size={15} />}
                onClick={() => void stopBuffer()}
                disabled={busy.stop || starting}
              >
                Stop
              </Button>
            </div>
          )}

          {errored && (
            <>
              <Button
                variant="primary"
                size="lg"
                icon={busy.start ? <Spinner size={18} /> : <Play size={18} />}
                onClick={() => void startBuffer()}
                disabled={busy.start || saving}
              >
                {busy.start ? "Starting…" : "Try again"}
              </Button>
              <p className="recorder__error" role="alert">
                {status.error ??
                  "Capture stopped before it started. Try again."}
              </p>
            </>
          )}
        </div>

        <div
          className="buffer"
          role="progressbar"
          aria-label="Replay buffer"
          aria-valuemin={0}
          aria-valuemax={target}
          aria-valuenow={buffered}
        >
          <div className="buffer__readout" aria-hidden="true">
            <span className="buffer__value">
              <span className="buffer__number">{buffered}</span>
              <span className="buffer__unit">s held</span>
            </span>
            <span className="buffer__target">{target} s</span>
          </div>
          <div className="buffer__track" aria-hidden="true">
            <span
              className="buffer__fill"
              style={{ width: `${fillPercent}%` }}
            />
          </div>
        </div>
      </div>

      <div className="recorder__quiet">
        {game && (
          <>
            <Badge tone="info" className="recorder__game">
              <Gamepad2 size={13} aria-hidden="true" />
              {game.name}
            </Badge>
            <span className="recorder__dot" aria-hidden="true">
              ·
            </span>
          </>
        )}
        <Switch
          compact
          className="recorder__toggle"
          label={`Mic ${micOn ? "on" : "off"}`}
          checked={micOn}
          disabled={busy.mic}
          onChange={() => void toggleMic()}
        />
        <span className="recorder__dot" aria-hidden="true">
          ·
        </span>
        <Switch
          compact
          className="recorder__toggle"
          label={`Desktop audio ${desktopOn ? "on" : "off"}`}
          checked={desktopOn}
          disabled={audioBusy || !settings}
          onChange={(next) => void setDesktopAudio(next)}
        />
        <span className="recorder__dot" aria-hidden="true">
          ·
        </span>
        <button
          type="button"
          className="recorder__quiet-link"
          onClick={() => openSettings("capture")}
          title="Open capture settings"
        >
          <span className="quiet-chip">
            <Cpu size={13} aria-hidden="true" />
            {encoder}
          </span>
          <span className="recorder__dot" aria-hidden="true">
            ·
          </span>
          <span className="quiet-chip">
            <Monitor size={13} aria-hidden="true" />
            {resolution}
          </span>
        </button>
      </div>

      {autoHint && (
        <p className="recorder__auto">
          <Gamepad2 size={14} aria-hidden="true" />
          Starts automatically when a game launches.
        </p>
      )}

      {audioError && <p className="recorder__error">{audioError}</p>}
      {statusError && (
        <p className="recorder__error recorder__error--muted">
          Recorder status unavailable: {statusError}
        </p>
      )}
    </section>
  );
}
