import {
  ChevronLeft,
  Pause,
  Play,
  ChevronRight,
  Film,
  Plus,
  Scissors,
  Trash2,
  Type,
  X,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";

import { Alert } from "../components/ui/Alert";
import { Button, IconButton } from "../components/ui/Button";
import { Slider } from "../components/ui/Slider";
import { Spinner } from "../components/ui/Spinner";
import { Switch } from "../components/ui/Switch";
import { backend, videoUrl } from "../lib/bridge";
import { errorMessage } from "../lib/errors";
import { formatDuration } from "../lib/format";
import type {
  Clip,
  EditPlan,
  EditorAspect,
  EditorOverlay,
  EditorSegment,
} from "../lib/types";
import { useApp } from "../state/store";

const clamp = (value: number, min: number, max: number) =>
  Math.min(max, Math.max(min, value));

function outMs(segment: EditorSegment): number {
  return Math.max(1, Math.round((segment.endMs - segment.startMs) / segment.speed));
}

export function EditorScreen() {
  const { editorClip } = useApp();
  if (!editorClip) {
    return (
      <div className="page">
        <Alert tone="warn" title="Nothing to edit">
          Open a clip and choose Edit clip to start an edit.
        </Alert>
      </div>
    );
  }
  return <Editor key={editorClip.id} clip={editorClip} />;
}

function Editor({ clip }: { clip: Clip }) {
  const { clips, mediaBase, closeEditor, addClip, openPlayer, pushToast } = useApp();
  const clipById = useMemo(() => new Map(clips.map((item) => [item.id, item])), [clips]);

  const [plan, setPlan] = useState<EditPlan>(() => ({
    title: "",
    aspect: "source",
    segments: [
      {
        clipId: clip.id,
        startMs: 0,
        endMs: clip.durationMs,
        speed: 1,
        desktopGain: 1,
        micGain: 1,
        muteDesktop: false,
        muteMic: false,
        fadeInMs: 0,
        fadeOutMs: 0,
        rotation: 0,
      },
    ],
    overlays: [],
  }));
  const [selected, setSelected] = useState<number | null>(null);
  const [playhead, setPlayhead] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [addOpen, setAddOpen] = useState(false);
  const [previewScale, setPreviewScale] = useState(1);
  const [stageBox, setStageBox] = useState({ width: 0, height: 0 });

  const videoRef = useRef<HTMLVideoElement>(null);
  const previewRef = useRef<HTMLDivElement>(null);
  const stageRef = useRef<HTMLDivElement>(null);

  const offsets: number[] = [];
  let total = 0;
  for (const segment of plan.segments) {
    offsets.push(total);
    total += outMs(segment);
  }

  const indexAt = (ms: number) => {
    let index = 0;
    for (let i = 0; i < plan.segments.length; i += 1) {
      if (offsets[i] <= ms) index = i;
    }
    return index;
  };

  const activeIndex = indexAt(playhead);
  const active = plan.segments[activeIndex];
  const activeClip = clipById.get(active.clipId);
  const outputHeight =
    plan.aspect === "vertical" ? 1920 : plan.aspect === "square" ? 1080 : (activeClip?.height ?? 1080);
  const sourceAt = (segment: EditorSegment, atMs: number) =>
    segment.startMs + (atMs - offsets[plan.segments.indexOf(segment)]) * segment.speed;
  const activeSourceMs = clamp(
    active.startMs + (playhead - offsets[activeIndex]) * active.speed,
    active.startMs,
    active.endMs,
  );

  // Keep the preview element's text scale in step with its rendered size.
  useEffect(() => {
    const node = previewRef.current;
    if (!node) return;
    const observer = new ResizeObserver(() => {
      setPreviewScale(node.clientHeight / Math.max(outputHeight, 1));
    });
    observer.observe(node);
    return () => observer.disconnect();
  }, [outputHeight]);

  useEffect(() => {
    const node = stageRef.current;
    if (!node) return;
    const observer = new ResizeObserver(() => {
      setStageBox({ width: node.clientWidth, height: node.clientHeight });
    });
    observer.observe(node);
    return () => observer.disconnect();
  }, []);

  const ratio =
    plan.aspect === "vertical"
      ? 9 / 16
      : plan.aspect === "square"
        ? 1
        : activeClip && activeClip.height > 0
          ? activeClip.width / activeClip.height
          : 16 / 9;
  const availableHeight = Math.max(160, (stageBox.height || 520) - 126);
  const previewWidth = Math.max(200, Math.min(stageBox.width || 900, availableHeight * ratio));
  const previewHeight = previewWidth / ratio;

  useEffect(() => {
    const video = videoRef.current;
    if (video) video.playbackRate = active.speed;
  }, [active.speed, activeIndex]);

  function seek(ms: number) {
    const next = clamp(ms, 0, total);
    setPlayhead(next);
    const index = indexAt(next);
    if (index === activeIndex && videoRef.current) {
      const segment = plan.segments[index];
      const source = segment.startMs + (next - offsets[index]) * segment.speed;
      videoRef.current.currentTime = clamp(source, segment.startMs, segment.endMs) / 1000;
    }
  }

  function onLoadedMetadata() {
    const video = videoRef.current;
    if (!video) return;
    video.currentTime = activeSourceMs / 1000;
    video.playbackRate = active.speed;
    if (playing) void video.play().catch(() => setPlaying(false));
  }

  function onTimeUpdate() {
    const video = videoRef.current;
    if (!video) return;
    const sourceMs = video.currentTime * 1000;
    if (sourceMs >= active.endMs - 15) {
      if (activeIndex + 1 < plan.segments.length) {
        setPlayhead(offsets[activeIndex + 1]);
      } else {
        video.pause();
        setPlaying(false);
        setPlayhead(total);
      }
      return;
    }
    // The video plays at `speed`, so source milliseconds divided by the speed
    // give output milliseconds.
    const mapped = offsets[activeIndex] + (sourceMs - active.startMs) / active.speed;
    setPlayhead(clamp(mapped, 0, total));
  }

  function patchSegment(change: Partial<EditorSegment>) {
    if (selected === null) return;
    setPlan((current) => {
      const segments = current.segments.map((segment, index) =>
        index === selected ? { ...segment, ...change } : segment,
      );
      return { ...current, segments };
    });
  }

  function splitAtPlayhead() {
    const index = indexAt(playhead);
    const segment = plan.segments[index];
    const cut = Math.round(segment.startMs + (playhead - offsets[index]) * segment.speed);
    if (cut <= segment.startMs + 80 || cut >= segment.endMs - 80) {
      pushToast("info", "Move the playhead away from the segment edge to split it.");
      return;
    }
    const segments = [...plan.segments];
    segments.splice(
      index,
      1,
      { ...segment, endMs: cut, fadeOutMs: 0 },
      { ...segment, startMs: cut, fadeInMs: 0 },
    );
    setPlan({ ...plan, segments });
    setSelected(index + 1);
  }

  function removeSegment() {
    if (selected === null || plan.segments.length <= 1) return;
    const segments = plan.segments.filter((_, index) => index !== selected);
    setPlan({ ...plan, segments });
    setSelected(null);
    setPlayhead(0);
  }

  function moveSegment(direction: -1 | 1) {
    if (selected === null) return;
    const target = selected + direction;
    if (target < 0 || target >= plan.segments.length) return;
    const segments = [...plan.segments];
    [segments[selected], segments[target]] = [segments[target], segments[selected]];
    setPlan({ ...plan, segments });
    setSelected(target);
  }

  function appendClip(item: Clip) {
    const segment: EditorSegment = {
      clipId: item.id,
      startMs: 0,
      endMs: item.durationMs,
      speed: 1,
      desktopGain: 1,
      micGain: 1,
      muteDesktop: false,
      muteMic: false,
      fadeInMs: 0,
      fadeOutMs: 0,
      rotation: 0,
    };
    setPlan((current) => ({ ...current, segments: [...current.segments, segment] }));
    setSelected(plan.segments.length);
    setAddOpen(false);
  }

  function patchOverlay(index: number, change: Partial<EditorOverlay>) {
    setPlan((current) => ({
      ...current,
      overlays: current.overlays.map((overlay, i) =>
        i === index ? { ...overlay, ...change } : overlay,
      ),
    }));
  }

  function addOverlay() {
    setPlan((current) => ({
      ...current,
      overlays: [
        ...current.overlays,
        {
          text: "Text",
          startMs: Math.round(playhead),
          endMs: Math.round(Math.min(total, playhead + 3000)),
          x: 0.5,
          y: 0.82,
          size: 48,
          color: "#ffffff",
          boxed: true,
        },
      ],
    }));
  }

  async function save() {
    setSaving(true);
    setError(null);
    try {
      const rendered = await backend.renderEdit({ ...plan, title: plan.title.trim() });
      addClip(rendered);
      pushToast("success", `Edit saved - ${rendered.title}`);
      openPlayer(rendered);
      closeEditor();
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setSaving(false);
    }
  }

  const selectedSegment = selected === null ? null : plan.segments[selected];
  const selectedClip = selectedSegment ? clipById.get(selectedSegment.clipId) : undefined;

  return (
    <div className="page editor">
      <header className="editor__head">
        <Button variant="ghost" icon={<ChevronLeft size={16} />} onClick={closeEditor}>
          Library
        </Button>
        <input
          className="editor__title"
          value={plan.title}
          placeholder={`${clip.title} (edit)`}
          aria-label="Edit title"
          onChange={(event) => setPlan({ ...plan, title: event.target.value })}
        />
        <span className="editor__duration">{formatDuration(total)}</span>
        <Button
          variant="primary"
          icon={saving ? <Spinner /> : <Film size={16} />}
          disabled={saving}
          onClick={() => void save()}
        >
          {saving ? "Rendering…" : "Save edit"}
        </Button>
      </header>

      {error && (
        <Alert tone="warn" title="The edit could not be rendered">
          {error}
        </Alert>
      )}

      <div className="editor__body">
        <div className="editor__stage" ref={stageRef}>
          <div
            className="editor__preview"
            ref={previewRef}
            style={{ width: previewWidth, height: previewHeight }}
          >
            {activeClip ? (
              <video
                key={activeIndex}
                ref={videoRef}
                src={videoUrl(activeClip, mediaBase)}
                controls
                playsInline
                onLoadedMetadata={onLoadedMetadata}
                onTimeUpdate={onTimeUpdate}
                onPlay={() => setPlaying(true)}
                onPause={() => setPlaying(false)}
                onEnded={() => setPlaying(false)}
              />
            ) : (
              <p className="editor__missing">The source clip is missing from the library.</p>
            )}
            {plan.overlays
              .filter((overlay) => playhead >= overlay.startMs && playhead <= overlay.endMs)
              .map((overlay, index) => (
                <span
                  key={`${overlay.text}-${index}`}
                  className="editor-overlay"
                  style={{
                    left: `${overlay.x * 100}%`,
                    top: `${overlay.y * 100}%`,
                    color: overlay.color,
                    fontSize: `${Math.max(9, overlay.size * previewScale)}px`,
                    background: overlay.boxed ? "rgba(0,0,0,0.55)" : undefined,
                    padding: overlay.boxed ? "0.15em 0.4em" : undefined,
                    transform: `translate(-${overlay.x * 100}%, -${overlay.y * 100}%)`,
                  }}
                >
                  {overlay.text}
                </span>
              ))}
          </div>

          <div className="editor__toolbar">
            <Button
              size="sm"
              variant="secondary"
              icon={playing ? <Pause size={15} /> : <Play size={15} />}
              onClick={() => {
                const video = videoRef.current;
                if (!video) return;
                if (playing) video.pause();
                else void video.play().catch(() => setPlaying(false));
              }}
            >
              {playing ? "Pause" : "Play"}
            </Button>
            <Button size="sm" variant="secondary" icon={<Plus size={15} />} onClick={() => setAddOpen(true)}>
              Add clip
            </Button>
            <Button size="sm" variant="ghost" icon={<Scissors size={15} />} onClick={splitAtPlayhead}>
              Split
            </Button>
            <IconButton
              label="Move segment left"
              disabled={selected === null || selected === 0}
              onClick={() => moveSegment(-1)}
            >
              <ChevronLeft size={16} />
            </IconButton>
            <IconButton
              label="Move segment right"
              disabled={selected === null || selected >= plan.segments.length - 1}
              onClick={() => moveSegment(1)}
            >
              <ChevronRight size={16} />
            </IconButton>
            <IconButton
              label="Delete segment"
              variant="danger"
              disabled={selected === null || plan.segments.length <= 1}
              onClick={removeSegment}
            >
              <Trash2 size={16} />
            </IconButton>
            <span className="editor__hint">
              {formatDuration(playhead)} / {formatDuration(total)}
            </span>
          </div>

          <div
            className="editor__timeline"
            role="slider"
            aria-label="Timeline"
            aria-valuemin={0}
            aria-valuemax={total}
            aria-valuenow={Math.round(playhead)}
            tabIndex={0}
            onClick={(event) => {
              const box = event.currentTarget.getBoundingClientRect();
              seek(((event.clientX - box.left) / box.width) * total);
            }}
          >
            <div className="editor__segments">
              {plan.segments.map((segment, index) => {
                const duration = outMs(segment);
                const source = clipById.get(segment.clipId);
                return (
                  <button
                    key={`${segment.clipId}-${index}`}
                    type="button"
                    className={`timeline-seg${index === selected ? " is-selected" : ""}${index === activeIndex ? " is-active" : ""}`}
                    style={{ width: `${(duration / Math.max(total, 1)) * 100}%` }}
                    title={`${source?.title ?? "Missing clip"} - ${formatDuration(duration)}`}
                    onClick={(event) => {
                      event.stopPropagation();
                      setSelected(index);
                      seek(offsets[index]);
                    }}
                  >
                    <span className="timeline-seg__title">
                      {source?.title ?? "Missing clip"}
                    </span>
                    <span className="timeline-seg__meta">
                      {formatDuration(duration)}
                      {segment.speed !== 1 ? ` · ${segment.speed}x` : ""}
                    </span>
                  </button>
                );
              })}
              <span
                className="editor__playhead"
                style={{ left: `${(playhead / Math.max(total, 1)) * 100}%` }}
                aria-hidden="true"
              />
            </div>
          </div>
        </div>

        <aside className="editor__side">
          <section className="editor-card">
            <h2 className="editor-card__title">Canvas</h2>
            <div className="editor-aspect">
              {(
                [
                  ["source", "Source"],
                  ["vertical", "9:16"],
                  ["square", "1:1"],
                ] as Array<[EditorAspect, string]>
              ).map(([value, label]) => (
                <button
                  key={value}
                  type="button"
                  className={`editor-aspect__option${plan.aspect === value ? " is-on" : ""}`}
                  onClick={() => setPlan({ ...plan, aspect: value })}
                >
                  {label}
                </button>
              ))}
            </div>
          </section>

          {selected === null || !selectedSegment ? (
            <section className="editor-card">
              <h2 className="editor-card__title">Segment</h2>
              <p className="editor-card__note">
                Select a segment on the timeline to change its range, speed, framing and audio.
              </p>
            </section>
          ) : (
            <>
          <section className="editor-card">
            <h2 className="editor-card__title">Segment {selected + 1}</h2>
            {selectedClip ? (
              <>
                <p className="editor-card__source">{selectedClip.title}</p>
                <div className="editor-grid">
                  <NumberField
                    label="In (s)"
                    value={selectedSegment.startMs / 1000}
                    min={0}
                    max={selectedSegment.endMs / 1000 - 0.1}
                    step={0.1}
                    onChange={(value) =>
                      patchSegment({ startMs: Math.round(clamp(value, 0, selectedSegment.endMs / 1000 - 0.1) * 1000) })
                    }
                  />
                  <NumberField
                    label="Out (s)"
                    value={selectedSegment.endMs / 1000}
                    min={selectedSegment.startMs / 1000 + 0.1}
                    max={selectedClip.durationMs / 1000}
                    step={0.1}
                    onChange={(value) =>
                      patchSegment({
                        endMs: Math.round(
                          clamp(value, selectedSegment.startMs / 1000 + 0.1, selectedClip.durationMs / 1000) * 1000,
                        ),
                      })
                    }
                  />
                </div>
                <div className="editor-row">
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() =>
                      patchSegment({
                        startMs: Math.round(clamp(sourceAt(selectedSegment, playhead), 0, selectedSegment.endMs - 100)),
                      })
                    }
                  >
                    In at playhead
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() =>
                      patchSegment({
                        endMs: Math.round(clamp(sourceAt(selectedSegment, playhead), selectedSegment.startMs + 100, selectedClip.durationMs)),
                      })
                    }
                  >
                    Out at playhead
                  </Button>
                </div>
                <div className="editor-grid">
                  <label className="editor-field">
                    <span>Speed</span>
                    <select
                      value={selectedSegment.speed}
                      onChange={(event) => patchSegment({ speed: Number(event.target.value) })}
                    >
                      {[0.5, 0.75, 1, 1.25, 1.5, 2].map((speed) => (
                        <option key={speed} value={speed}>
                          {speed}x
                        </option>
                      ))}
                    </select>
                  </label>
                  <label className="editor-field">
                    <span>Rotation</span>
                    <select
                      value={selectedSegment.rotation}
                      onChange={(event) =>
                        patchSegment({ rotation: Number(event.target.value) as EditorSegment["rotation"] })
                      }
                    >
                      {[0, 90, 180, 270].map((rotation) => (
                        <option key={rotation} value={rotation}>
                          {rotation}°
                        </option>
                      ))}
                    </select>
                  </label>
                  <NumberField
                    label="Fade in (ms)"
                    value={selectedSegment.fadeInMs}
                    min={0}
                    max={outMs(selectedSegment)}
                    step={100}
                    onChange={(value) => patchSegment({ fadeInMs: Math.round(value) })}
                  />
                  <NumberField
                    label="Fade out (ms)"
                    value={selectedSegment.fadeOutMs}
                    min={0}
                    max={outMs(selectedSegment)}
                    step={100}
                    onChange={(value) => patchSegment({ fadeOutMs: Math.round(value) })}
                  />
                </div>
              </>
            ) : (
              <p className="editor-card__source">Missing clip</p>
            )}
          </section>

          <section className="editor-card">
            <h2 className="editor-card__title">Audio</h2>
            <Switch
              compact
              label="Desktop audio"
              checked={!selectedSegment.muteDesktop}
              onChange={(next) => patchSegment({ muteDesktop: !next })}
            />
            <Slider
              label="Game volume"
              min={0}
              max={2}
              step={0.05}
              value={selectedSegment.desktopGain}
              formatValue={(value) => `${Math.round(value * 100)}%`}
              disabled={selectedSegment.muteDesktop}
              onChange={(value) => patchSegment({ desktopGain: value })}
            />
            <Switch
              compact
              label="Microphone"
              checked={!selectedSegment.muteMic}
              onChange={(next) => patchSegment({ muteMic: !next })}
            />
            <Slider
              label="Mic volume"
              min={0}
              max={2}
              step={0.05}
              value={selectedSegment.micGain}
              formatValue={(value) => `${Math.round(value * 100)}%`}
              disabled={selectedSegment.muteMic}
              onChange={(value) => patchSegment({ micGain: value })}
            />
            <p className="editor-card__note">
              Fades apply to both tracks. The saved clip keeps the mixed track plus the
              isolated game and microphone tracks.
            </p>
          </section>
            </>
          )}

          <section className="editor-card">
            <div className="editor-card__head">
              <h2 className="editor-card__title">Text</h2>
              <Button size="sm" variant="ghost" icon={<Type size={14} />} onClick={addOverlay}>
                Add
              </Button>
            </div>
            {plan.overlays.length === 0 && (
              <p className="editor-card__note">No text overlays yet.</p>
            )}
            {plan.overlays.map((overlay, index) => (
              <div className="overlay-row" key={index}>
                <input
                  className="overlay-row__text"
                  value={overlay.text}
                  aria-label="Overlay text"
                  onChange={(event) => patchOverlay(index, { text: event.target.value })}
                />
                <div className="editor-grid editor-grid--tight">
                  <NumberField
                    label="Start (s)"
                    value={overlay.startMs / 1000}
                    min={0}
                    max={total / 1000}
                    step={0.1}
                    onChange={(value) =>
                      patchOverlay(index, { startMs: Math.round(clamp(value, 0, total / 1000) * 1000) })
                    }
                  />
                  <NumberField
                    label="End (s)"
                    value={overlay.endMs / 1000}
                    min={0}
                    max={total / 1000}
                    step={0.1}
                    onChange={(value) =>
                      patchOverlay(index, { endMs: Math.round(clamp(value, 0, total / 1000) * 1000) })
                    }
                  />
                  <NumberField
                    label="X"
                    value={overlay.x}
                    min={0}
                    max={1}
                    step={0.05}
                    onChange={(value) => patchOverlay(index, { x: clamp(value, 0, 1) })}
                  />
                  <NumberField
                    label="Y"
                    value={overlay.y}
                    min={0}
                    max={1}
                    step={0.05}
                    onChange={(value) => patchOverlay(index, { y: clamp(value, 0, 1) })}
                  />
                  <NumberField
                    label="Size"
                    value={overlay.size}
                    min={8}
                    max={200}
                    step={1}
                    onChange={(value) => patchOverlay(index, { size: Math.round(value) })}
                  />
                  <label className="editor-field">
                    <span>Colour</span>
                    <input
                      type="color"
                      value={overlay.color}
                      onChange={(event) => patchOverlay(index, { color: event.target.value })}
                    />
                  </label>
                </div>
                <div className="editor-row">
                  <Switch
                    compact
                    label="Backdrop"
                    checked={overlay.boxed}
                    onChange={(next) => patchOverlay(index, { boxed: next })}
                  />
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() =>
                      patchOverlay(index, { startMs: Math.round(playhead) })
                    }
                  >
                    Start here
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() =>
                      patchOverlay(index, { endMs: Math.round(playhead) })
                    }
                  >
                    End here
                  </Button>
                  <IconButton
                    label="Remove overlay"
                    variant="danger"
                    onClick={() =>
                      setPlan((current) => ({
                        ...current,
                        overlays: current.overlays.filter((_, i) => i !== index),
                      }))
                    }
                  >
                    <X size={14} />
                  </IconButton>
                </div>
              </div>
            ))}
          </section>
        </aside>
      </div>

      {addOpen && (
        <div className="editor-picker" role="dialog" aria-label="Add a clip">
          <div className="editor-picker__head">
            <h2>Add a clip</h2>
            <IconButton label="Close clip picker" onClick={() => setAddOpen(false)}>
              <X size={16} />
            </IconButton>
          </div>
          <div className="editor-picker__list">
            {clips.map((item) => (
              <button
                key={item.id}
                type="button"
                className="editor-picker__item"
                onClick={() => appendClip(item)}
              >
                <span>{item.title}</span>
                <span>
                  {item.game ?? "Desktop"} · {formatDuration(item.durationMs)}
                </span>
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function NumberField({
  label,
  value,
  min,
  max,
  step,
  onChange,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  onChange: (value: number) => void;
}) {
  return (
    <label className="editor-field">
      <span>{label}</span>
      <input
        type="number"
        value={Number.isFinite(value) ? Number(value.toFixed(2)) : 0}
        min={min}
        max={max}
        step={step}
        onChange={(event) => {
          const next = Number(event.target.value);
          if (Number.isFinite(next)) onChange(next);
        }}
      />
    </label>
  );
}
