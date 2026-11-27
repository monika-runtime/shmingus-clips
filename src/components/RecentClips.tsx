import {
  ArrowRight,
  Download,
  Film,
  FolderOpen,
  Pencil,
  Play,
  Share2,
  Star,
  Trash2,
} from "lucide-react";
import { useState, type MouseEvent as ReactMouseEvent } from "react";

import { formatDuration, formatRelative } from "../lib/format";
import type { Clip } from "../lib/types";
import { useContextMenu } from "../state/contextMenu";
import { useApp } from "../state/store";
import { useClipActions } from "../state/useClipActions";
import { useShareActions } from "../state/useShareActions";
import { ClipThumb } from "./ClipThumb";
import { Button } from "./ui/Button";
import { ConfirmDialog } from "./ui/ConfirmDialog";

function RecentItem({
  clip,
  onOpen,
}: {
  clip: Clip;
  onOpen: (clip: Clip) => void;
}) {
  const { requestRename, shares } = useApp();
  const { toggleFavorite, remove, exportClip, reveal } = useClipActions();
  const { share, uploadingId } = useShareActions();
  const { openContextMenu } = useContextMenu();
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [exporting, setExporting] = useState(false);
  const shared = Boolean(shares[clip.id]);
  const sharing = uploadingId === clip.id;

  function openMenu(event: ReactMouseEvent<HTMLElement>) {
    event.preventDefault();
    event.stopPropagation();
    openContextMenu(event, [
      { heading: clip.title },
      { label: "Open clip", icon: Play, onSelect: () => onOpen(clip) },
      {
        label: clip.favorite ? "Remove from favorites" : "Add to favorites",
        icon: Star,
        onSelect: () => void toggleFavorite(clip),
      },
      { label: "Rename", icon: Pencil, onSelect: () => requestRename(clip) },
      {
        label: "Reveal file",
        icon: FolderOpen,
        onSelect: () => void reveal(clip),
      },
      {
        label: "Export",
        icon: Download,
        disabled: exporting,
        onSelect: () => {
          setExporting(true);
          void exportClip(clip).finally(() => setExporting(false));
        },
      },
      {
        label: sharing ? "Uploading…" : shared ? "Manage share" : "Share",
        icon: Share2,
        disabled: sharing,
        onSelect: () => void share(clip),
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

  return (
    <>
      <button
        type="button"
        className="recent__item"
        onClick={() => onOpen(clip)}
        onContextMenu={openMenu}
        aria-label={`Open ${clip.title}`}
      >
        <ClipThumb clip={clip} className="recent__thumb" />
        <span className="recent__title">{clip.title}</span>
        <span className="recent__meta">
          {formatDuration(clip.durationMs)} · {formatRelative(clip.createdAt)}
        </span>
      </button>

      {confirmOpen && (
        <ConfirmDialog
          title="Delete this clip?"
          message={`“${clip.title}” and its file will be removed from disk.${shared ? " Its public share link will be removed first." : ""} This cannot be undone.`}
          busy={removing}
          onCancel={() => setConfirmOpen(false)}
          onConfirm={() => {
            setRemoving(true);
            void remove(clip).then((ok) => {
              setRemoving(false);
              if (ok) setConfirmOpen(false);
            });
          }}
        />
      )}
    </>
  );
}

export function RecentClips() {
  const { clips, clipsLoading, openPlayer, navigate } = useApp();
  const recent = clips.slice(0, 4);

  return (
    <section className="section" aria-labelledby="recent-title">
      <div className="section__head">
        <div>
          <p className="micro-label">Library</p>
          <h2 className="section__title" id="recent-title">
            Recent clips
          </h2>
        </div>
        <Button
          variant="ghost"
          size="sm"
          trailing={<ArrowRight size={15} />}
          onClick={() => navigate("library")}
        >
          View all
        </Button>
      </div>

      {clipsLoading && recent.length === 0 ? (
        <div className="recent">
          {Array.from({ length: 4 }, (_, index) => (
            <div key={index} className="recent__item skeleton" />
          ))}
        </div>
      ) : recent.length > 0 ? (
        <div className="recent">
          {recent.map((clip) => (
            <RecentItem key={clip.id} clip={clip} onOpen={openPlayer} />
          ))}
        </div>
      ) : (
        <div className="recent__empty">
          <span className="recent__empty-icon" aria-hidden="true">
            <Film size={20} />
          </span>
          <div>
            <p className="recent__empty-title">No clips yet</p>
            <p className="recent__empty-text">
              Hit Save clip while the buffer is running and your first replay
              lands here.
            </p>
          </div>
        </div>
      )}
    </section>
  );
}
