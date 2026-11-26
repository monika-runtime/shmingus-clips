import {
  Download,
  FolderOpen,
  Pause,
  Pencil,
  Play,
  Scissors,
  Share2,
  Star,
  Trash2,
  X,
} from "lucide-react";
import { useEffect, useId, useRef, useState, type MouseEvent as ReactMouseEvent } from "react";

import { ClipThumb } from "../components/ClipThumb";
import { TrimTimeline } from "../components/TrimTimeline";
import { Alert } from "../components/ui/Alert";
import { Button, IconButton } from "../components/ui/Button";
import { ConfirmDialog } from "../components/ui/ConfirmDialog";
import { Modal } from "../components/ui/Modal";
import { RenameDialog } from "../components/ui/RenameDialog";
import { Spinner } from "../components/ui/Spinner";
import { backend, isDesktop, mediaUrl, videoUrl } from "../lib/bridge";
import { clamp, formatBytes, formatDateTime, formatDuration, formatResolution } from "../lib/format";
import { errorMessage } from "../lib/errors";
import type { Clip } from "../lib/types";
import { isNativeContextTarget, useContextMenu } from "../state/contextMenu";
import { useApp } from "../state/store";
import { useClipActions } from "../state/useClipActions";
import { useShareActions } from "../state/useShareActions";

export function PlayerModal() {
  const { playerClip } = useApp();
  if (!playerClip) return null;
  return <PlayerDialog key={playerClip.id} clip={playerClip} />;
}

function PlayerDialog({ clip }: { clip: Clip }) {
  const {
    closePlayer,
    addClip,
    openPlayer,
    pushToast,
    mediaBase,
    shares,
    setShare,
  } = useApp();
  const { toggleFavorite, rename, remove, exportClip, showExportFolder, reveal } =
    useClipActions();
  const { share, uploadingId } = useShareActions();
  const { openContextMenu } = useContextMenu();
  const [mode, setMode] = useState<"view" | "trim">("view");
  const [startMs, setStartMs] = useState(0);
  const [endMs, setEndMs] = useState(clip.durationMs);
  const [playheadMs, setPlayheadMs] = useState(0);
  const [previewPlaying, setPreviewPlaying] = useState(false);
  const [trimming, setTrimming] = useState(false);
  const [trimError, setTrimError] = useState<string | null>(null);
  const [mediaError, setMediaError] = useState(false);
  const [renameOpen, setRenameOpen] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [exportedPath, setExportedPath] = useState<string | null>(null);
  const [busy, setBusy] = useState<"rename" | "delete" | "export" | null>(null);
  const videoRef = useRef<HTMLVideoElement>(null);
  const titleId = useId();

  const desktop = isDesktop();
  const duration = Math.max(clip.durationMs, 1);
  const selectedMs = Math.max(0, endMs - startMs);
  const canTrim = duration >= 100;
  const shared = Boolean(shares[clip.id]);
  const sharing = uploadingId === clip.id;

  // The store can lag behind a share made before this window opened; ask the
  // backend once so the button shows the right state.
  useEffect(() => {
    if (shares[clip.id]) return;
    let active = true;
    backend
      .getShare(clip.id)
      .then((info) => {
        if (active && info) setShare(clip.id, info);
      })
      .catch(() => {
        // The share simply is not known yet.
      });
    return () => {
      active = false;
    };
  }, [clip.id, shares, setShare]);

  useEffect(() => {
    if (desktop || !previewPlaying) return;
    const timer = window.setInterval(() => {
      setPlayheadMs((current) =>
        current + 100 >= duration ? 0 : current + 100,
      );
    }, 100);
    return () => window.clearInterval(timer);
  }, [desktop, previewPlaying, duration]);

  function seek(ms: number) {
    const next = clamp(ms, 0, duration);
    setPlayheadMs(next);
    const video = videoRef.current;
    if (video && Number.isFinite(video.duration) && video.duration > 0) {
      video.currentTime = clamp(next / 1000, 0, video.duration);
    }
  }

  function enterTrim() {
    videoRef.current?.pause();
    setMode("trim");
    setStartMs(0);
    setEndMs(duration);
    setTrimError(null);
    seek(0);
  }

  function leaveTrim() {
    setMode("view");
    setPreviewPlaying(false);
  }

  function togglePlayback() {
    const video = videoRef.current;
    if (video) {
      if (video.paused) {
        void video.play().catch(() => {
          // The media element can refuse playback; the native controls remain.
        });
      } else {
        video.pause();
      }
      return;
    }
    setPreviewPlaying((current) => !current);
  }

  function exportNow() {
    setBusy("export");
    void exportClip(clip)
      .then((path) => {
        if (path) setExportedPath(path);
      })
      .finally(() => setBusy(null));
  }

  function openMenu(event: ReactMouseEvent<HTMLDivElement>) {
    if (isNativeContextTarget(event.target)) return;
    event.preventDefault();
    event.stopPropagation();
    const video = videoRef.current;
    const playing = video ? !video.paused : previewPlaying;
    openContextMenu(event, [
      {
        label: playing ? "Pause" : "Play",
        icon: playing ? Pause : Play,
        onSelect: togglePlayback,
      },
      {
        label: mode === "trim" ? "Done trimming" : "Trim clip",
        icon: Scissors,
        disabled: !canTrim,
        onSelect: () => (mode === "trim" ? leaveTrim() : enterTrim()),
      },
      {
        label: clip.favorite ? "Remove from favorites" : "Add to favorites",
        icon: Star,
        onSelect: () => void toggleFavorite(clip),
      },
      {
        label: "Rename",
        icon: Pencil,
        onSelect: () => setRenameOpen(true),
      },
      {
        label: "Export",
        icon: Download,
        disabled: busy !== null,
        onSelect: exportNow,
      },
      {
        label: sharing ? "Uploading…" : shared ? "Share link" : "Share",
        icon: Share2,
        disabled: busy !== null || sharing,
        onSelect: () => void share(clip),
      },
      {
        label: "Reveal file",
        icon: FolderOpen,
        onSelect: () => void reveal(clip),
      },
      { separator: true },
      {
        label: "Delete",
        icon: Trash2,
        danger: true,
        onSelect: () => setConfirmOpen(true),
      },
    ]);
  }

  async function saveTrim() {
    if (selectedMs < 100) {
      setTrimError("Select at least 0.1 seconds.");
      return;
    }
    setTrimming(true);
    setTrimError(null);
    try {
      const trimmed = await backend.trimClip(clip.id, startMs, endMs);
      addClip(trimmed);
      openPlayer(trimmed);
      pushToast("success", `Trim saved - ${trimmed.title}`);
    } catch (error) {
      const message = errorMessage(error);
      setTrimError(message);
      pushToast("error", message);
    } finally {
      setTrimming(false);
    }
  }

  return (
    <Modal
      onClose={closePlayer}
      labelledBy={titleId}
      className="player"
      onContextMenu={openMenu}
    >
      <header className="player__head">
        <div className="player__heading">
          <h2 className="player__title" id={titleId}>
            {clip.title}
          </h2>
        </div>
        <div className="player__head-actions">
          <IconButton
            label={clip.favorite ? "Remove from favorites" : "Add to favorites"}
            className={clip.favorite ? "is-on" : undefined}
            onClick={() => void toggleFavorite(clip)}
          >
            <Star size={18} fill={clip.favorite ? "currentColor" : "none"} />
          </IconButton>
          <IconButton label="Close player" onClick={closePlayer}>
            <X size={18} />
          </IconButton>
        </div>
      </header>

      <div className="player__body">
        <div className="player__stage-wrap">
          <div className="player__stage">
            {desktop && clip.path ? (
              <video
                ref={videoRef}
                className="player__video"
                src={videoUrl(clip, mediaBase) || mediaUrl(clip.path)}
                poster={clip.thumbnail ? mediaUrl(clip.thumbnail) : undefined}
                controls
                autoPlay
                playsInline
                onTimeUpdate={(event) => {
                  setPlayheadMs(event.currentTarget.currentTime * 1000);
                }}
                onError={() => setMediaError(true)}
              />
            ) : (
              <div className="player__preview">
                <ClipThumb clip={clip} className="player__poster" />
                <div className="player__preview-bar">
                  <button
                    type="button"
                    className="player__preview-play"
                    aria-label={previewPlaying ? "Pause preview" : "Play preview"}
                    onClick={() => setPreviewPlaying((current) => !current)}
                  >
                    {previewPlaying ? <Pause size={15} /> : <Play size={15} />}
                  </button>
                  <div className="player__preview-track" aria-hidden="true">
                    <span
                      style={{
                        width: `${clamp((playheadMs / duration) * 100, 0, 100)}%`,
                      }}
                    />
                  </div>
                  <span className="player__preview-time">
                    {formatDuration(playheadMs)} / {formatDuration(duration)}
                  </span>
                </div>
                <p className="player__preview-note">
                  {desktop
                    ? "This clip has no media path."
                    : "Design preview - video decodes in the desktop app."}
                </p>
              </div>
            )}
            {mediaError && (
              <Alert tone="warn" className="player__media-error">
                The media file could not be loaded. It may have moved since it
                was indexed.
              </Alert>
            )}
          </div>

          {mode === "trim" && (
            <section className="trim" aria-labelledby="trim-title">
              <div className="trim__head">
                <div>
                  <p className="micro-label">Trim</p>
                  <h3 className="trim__title" id="trim-title">
                    Cut a section
                  </h3>
                </div>
                <span className="trim__readout">
                  {formatDuration(startMs)} → {formatDuration(endMs)}
                  <span className="trim__readout-total">
                    {formatDuration(selectedMs)} selected
                  </span>
                </span>
              </div>

              <TrimTimeline
                durationMs={duration}
                startMs={startMs}
                endMs={endMs}
                playheadMs={playheadMs}
                onChange={({ startMs: nextStart, endMs: nextEnd }) => {
                  setStartMs(nextStart);
                  setEndMs(nextEnd);
                }}
                onSeek={seek}
                disabled={trimming}
              />

              <div className="trim__footer">
                <div className="trim__values">
                  <span>
                    <span className="micro-label">In</span>{" "}
                    {formatDuration(startMs)}
                  </span>
                  <span>
                    <span className="micro-label">Out</span>{" "}
                    {formatDuration(endMs)}
                  </span>
                </div>
                <div className="trim__buttons">
                  <Button
                    variant="ghost"
                    disabled={trimming}
                    onClick={() => {
                      setStartMs(0);
                      setEndMs(duration);
                      setTrimError(null);
                    }}
                  >
                    Reset
                  </Button>
                  <Button
                    variant="primary"
                    icon={trimming ? <Spinner /> : <Scissors size={16} />}
                    disabled={trimming || selectedMs < 100}
                    onClick={() => void saveTrim()}
                  >
                    {trimming ? "Saving…" : "Save trimmed clip"}
                  </Button>
                </div>
              </div>
              {trimError && <Alert tone="danger">{trimError}</Alert>}
            </section>
          )}
        </div>

        <aside className="player__side">
          <dl className="player__meta">
            <div>
              <dt>Game</dt>
              <dd>{clip.game ?? "Desktop"}</dd>
            </div>
            <div>
              <dt>Recorded</dt>
              <dd>{formatDateTime(clip.createdAt)}</dd>
            </div>
            <div>
              <dt>Duration</dt>
              <dd>{formatDuration(clip.durationMs)}</dd>
            </div>
            <div>
              <dt>Resolution</dt>
              <dd>{formatResolution(clip.width, clip.height)}</dd>
            </div>
            <div>
              <dt>Size</dt>
              <dd>{formatBytes(clip.sizeBytes)}</dd>
            </div>
          </dl>

          <div className="player__actions">
            <Button
              variant={mode === "trim" ? "primary" : "secondary"}
              icon={<Scissors size={16} />}
              disabled={!canTrim}
              onClick={() => (mode === "trim" ? leaveTrim() : enterTrim())}
            >
              {mode === "trim" ? "Done trimming" : "Trim clip"}
            </Button>
            <Button
              icon={busy === "export" ? <Spinner /> : <Download size={16} />}
              disabled={busy !== null}
              onClick={() => {
                setBusy("export");
                void exportClip(clip)
                  .then((path) => {
                    if (path) setExportedPath(path);
                  })
                  .finally(() => setBusy(null));
              }}
            >
              Export
            </Button>
            <Button
              icon={sharing ? <Spinner /> : <Share2 size={16} />}
              disabled={busy !== null || sharing}
              onClick={() => void share(clip)}
            >
              {sharing ? "Uploading…" : shared ? "Share link" : "Share"}
            </Button>
            <Button
              icon={<FolderOpen size={16} />}
              disabled={busy !== null}
              onClick={() => void reveal(clip)}
            >
              Reveal file
            </Button>
            <Button
              icon={<Pencil size={16} />}
              disabled={busy !== null}
              onClick={() => setRenameOpen(true)}
            >
              Rename
            </Button>
            <Button
              variant="ghost"
              className="player__delete"
              icon={<Trash2 size={16} />}
              disabled={busy !== null}
              onClick={() => setConfirmOpen(true)}
            >
              Delete
            </Button>
          </div>

          {exportedPath && (
            <div className="player__export" role="status">
              <p className="player__export-title">Exported</p>
              <p className="player__export-path" title={exportedPath}>
                {exportedPath}
              </p>
              <Button
                size="sm"
                variant="secondary"
                icon={<FolderOpen size={15} />}
                onClick={() => void showExportFolder(exportedPath)}
              >
                Open folder
              </Button>
            </div>
          )}

          <p className="player__path" title={clip.path}>
            {clip.path}
          </p>
        </aside>
      </div>

      {renameOpen && (
        <RenameDialog
          initialTitle={clip.title}
          busy={busy === "rename"}
          onCancel={() => setRenameOpen(false)}
          onSubmit={(title) => {
            setBusy("rename");
            void rename(clip, title).then((updated) => {
              setBusy(null);
              if (updated) setRenameOpen(false);
            });
          }}
        />
      )}

      {confirmOpen && (
        <ConfirmDialog
          title="Delete this clip?"
          message={`“${clip.title}” and its file will be removed from disk.${shared ? " Its public share link will be removed first." : ""} This cannot be undone.`}
          busy={busy === "delete"}
          onCancel={() => setConfirmOpen(false)}
          onConfirm={() => {
            setBusy("delete");
            void remove(clip).then((ok) => {
              setBusy(null);
              if (ok) setConfirmOpen(false);
            });
          }}
        />
      )}
    </Modal>
  );
}
