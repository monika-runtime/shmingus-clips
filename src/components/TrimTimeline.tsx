import { useId, useRef, type KeyboardEvent, type PointerEvent } from "react";

import { clamp, formatDuration } from "../lib/format";

const MIN_GAP_MS = 100;
const SNAP_MS = 100;

interface TrimTimelineProps {
  durationMs: number;
  startMs: number;
  endMs: number;
  playheadMs: number;
  onChange: (next: { startMs: number; endMs: number }) => void;
  onSeek: (ms: number) => void;
  disabled?: boolean;
}

function snap(ms: number): number {
  return Math.round(ms / SNAP_MS) * SNAP_MS;
}

export function TrimTimeline({
  durationMs,
  startMs,
  endMs,
  playheadMs,
  onChange,
  onSeek,
  disabled,
}: TrimTimelineProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const hintId = useId();
  const span = Math.max(durationMs, 1);
  const startPercent = clamp((startMs / span) * 100, 0, 100);
  const endPercent = clamp((endMs / span) * 100, 0, 100);
  const playheadPercent = clamp((playheadMs / span) * 100, 0, 100);

  function snapFromClientX(clientX: number): number {
    const container = containerRef.current;
    if (!container) return 0;
    const rect = container.getBoundingClientRect();
    const ratio = clamp((clientX - rect.left) / rect.width, 0, 1);
    return snap(ratio * durationMs);
  }

  function beginDrag(which: "start" | "end", event: PointerEvent<HTMLDivElement>) {
    if (disabled) return;
    event.preventDefault();
    event.stopPropagation();
    const move = (native: globalThis.PointerEvent) => {
      const ms = snapFromClientX(native.clientX);
      if (which === "start") {
        onChange({ startMs: clamp(ms, 0, endMs - MIN_GAP_MS), endMs });
      } else {
        onChange({
          startMs,
          endMs: clamp(ms, startMs + MIN_GAP_MS, durationMs),
        });
      }
    };
    const stop = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", stop);
      window.removeEventListener("pointercancel", stop);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", stop);
    window.addEventListener("pointercancel", stop);
    move(event.nativeEvent);
  }

  function onHandleKey(
    which: "start" | "end",
    event: KeyboardEvent<HTMLDivElement>,
  ) {
    if (disabled) return;
    const step = event.shiftKey ? 1000 : 100;
    const apply = (delta: number) => {
      if (which === "start") {
        onChange({
          startMs: clamp(startMs + delta, 0, endMs - MIN_GAP_MS),
          endMs,
        });
      } else {
        onChange({
          startMs,
          endMs: clamp(endMs + delta, startMs + MIN_GAP_MS, durationMs),
        });
      }
    };
    let handled = true;

    switch (event.key) {
      case "ArrowLeft":
      case "ArrowDown":
        apply(-step);
        break;
      case "ArrowRight":
      case "ArrowUp":
        apply(step);
        break;
      case "PageDown":
        apply(-1000);
        break;
      case "PageUp":
        apply(1000);
        break;
      case "Home":
        if (which === "start") {
          onChange({ startMs: 0, endMs });
        } else {
          onChange({ startMs, endMs: startMs + MIN_GAP_MS });
        }
        break;
      case "End":
        if (which === "start") {
          onChange({ startMs: endMs - MIN_GAP_MS, endMs });
        } else {
          onChange({ startMs, endMs: durationMs });
        }
        break;
      default:
        handled = false;
    }

    if (handled) event.preventDefault();
  }

  return (
    <div
      className={disabled ? "timeline is-disabled" : "timeline"}
      ref={containerRef}
      onPointerDown={(event) => {
        if (disabled) return;
        onSeek(snapFromClientX(event.clientX));
      }}
      aria-hidden={disabled || undefined}
    >
      <div className="timeline__rail" aria-hidden="true" />
      <div
        className="timeline__selection"
        style={{
          left: `${startPercent}%`,
          width: `${Math.max(0, endPercent - startPercent)}%`,
        }}
        aria-hidden="true"
      />
      <div
        className="timeline__playhead"
        style={{ left: `${playheadPercent}%` }}
        aria-hidden="true"
      >
        <span className="timeline__playhead-dot" />
      </div>

      <div
        role="slider"
        tabIndex={disabled ? -1 : 0}
        aria-label="Trim start"
        aria-orientation="horizontal"
        aria-valuemin={0}
        aria-valuemax={Math.max(0, endMs - MIN_GAP_MS)}
        aria-valuenow={startMs}
        aria-valuetext={formatDuration(startMs)}
        aria-describedby={hintId}
        className="timeline__handle timeline__handle--start"
        style={{ left: `${startPercent}%` }}
        onPointerDown={(event) => beginDrag("start", event)}
        onKeyDown={(event) => onHandleKey("start", event)}
      >
        <span className="timeline__grip" aria-hidden="true" />
      </div>

      <div
        role="slider"
        tabIndex={disabled ? -1 : 0}
        aria-label="Trim end"
        aria-orientation="horizontal"
        aria-valuemin={Math.min(durationMs, startMs + MIN_GAP_MS)}
        aria-valuemax={durationMs}
        aria-valuenow={endMs}
        aria-valuetext={formatDuration(endMs)}
        aria-describedby={hintId}
        className="timeline__handle timeline__handle--end"
        style={{ left: `${endPercent}%` }}
        onPointerDown={(event) => beginDrag("end", event)}
        onKeyDown={(event) => onHandleKey("end", event)}
      >
        <span className="timeline__grip" aria-hidden="true" />
      </div>

      <p id={hintId} className="sr-only">
        Use the arrow keys to adjust the trim handles by 0.1 seconds, or hold
        Shift for 1 second. Click the timeline to preview-seek.
      </p>
    </div>
  );
}
