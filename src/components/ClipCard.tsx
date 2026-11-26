import {
  Download,
  FolderOpen,
  Gamepad2,
  Link2,
  Pencil,
  Play,
  Share2,
  Star,
  Trash2,
} from "lucide-react";
import { useState, type MouseEvent as ReactMouseEvent } from "react";

import { cx } from "../lib/cx";
import { formatBytes, formatDuration, formatRelative } from "../lib/format";
import type { Clip } from "../lib/types";
import { useContextMenu } from "../state/contextMenu";
import { useApp } from "../state/store";
import { useClipActions } from "../state/useClipActions";
import { useShareActions } from "../state/useShareActions";
import { ClipThumb } from "./ClipThumb";
import { Button, IconButton } from "./ui/Button";
import { ConfirmDialog } from "./ui/ConfirmDialog";
import { RenameDialog } from "./ui/RenameDialog";

interface ClipCardProps {
  clip: Clip;
  onOpen: (clip: Clip) => void;
}

export function ClipCard({ clip, onOpen }: ClipCardProps) {
  const { shares } = useApp();
  const { toggleFavorite, rename, remove, exportClip, reveal } = useClipActions();
  const { share, uploadingId } = useShareActions();
  const { openContextMenu } = useContextMenu();
  const [renameOpen, setRenameOpen] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [renaming, setRenaming] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [exporting, setExporting] = useState(false);

  const shared = Boolean(shares[clip.id]);
  const sharing = uploadingId === clip.id;

  function openMenu(event: ReactMouseEvent<HTMLElement>) {
    event.preventDefault();
    event.stopPropagation();
    openContextMenu(event, [
      { heading: clip.title },
      {
        label: "Open clip",
        icon: Play,
        onSelect: () => onOpen(clip),
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
        label: sharing ? "Uploading…" : shared ? "Share link" : "Share",
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
    <article
      className={cx("clip-card", clip.favorite && "is-favorite")}
      onContextMenu={openMenu}
    >
      <div className="clip-card__media">
        <button
          type="button"
          className="clip-card__open"
          onClick={() => onOpen(clip)}
          aria-label={`Open ${clip.title}`}
        >
          <ClipThumb clip={clip} />
          <span className="clip-card__duration">
            {formatDuration(clip.durationMs)}
          </span>
        </button>
        {shared && (
          <span className="clip-card__shared" title="Shared on your clip server">
            <Link2 size={11} aria-hidden="true" /> Shared
          </span>
        )}
        <IconButton
          label={clip.favorite ? "Remove from favorites" : "Add to favorites"}
          className={cx("clip-card__star", clip.favorite && "is-on")}
          onClick={() => void toggleFavorite(clip)}
        >
          <Star size={16} fill={clip.favorite ? "currentColor" : "none"} />
        </IconButton>
      </div>

      <div className="clip-card__body">
        <button
          type="button"
          className="clip-card__title"
          onClick={() => onOpen(clip)}
        >
          {clip.title}
        </button>
        <div className="clip-card__meta">
          <span className="clip-card__game">
            <Gamepad2 size={13} aria-hidden="true" />
            <span className="clip-card__game-name">
              {clip.game ?? "Desktop"}
            </span>
          </span>
          <span>{formatBytes(clip.sizeBytes)}</span>
          <span>{formatRelative(clip.createdAt)}</span>
        </div>
        <div className="clip-card__actions">
          <Button
            size="sm"
            variant="ghost"
            className="clip-card__rename"
            icon={<Pencil size={14} />}
            title="Rename clip"
            onClick={() => setRenameOpen(true)}
          >
            Rename
          </Button>
          <IconButton
            label="Reveal file"
            onClick={() => void reveal(clip)}
          >
            <FolderOpen size={15} />
          </IconButton>
          <IconButton
            label="Export clip"
            disabled={exporting}
            onClick={() => {
              setExporting(true);
              void exportClip(clip).finally(() => setExporting(false));
            }}
          >
            <Download size={15} />
          </IconButton>
          <IconButton
            label="Delete clip"
            variant="danger"
            className="clip-card__delete"
            onClick={() => setConfirmOpen(true)}
          >
            <Trash2 size={15} />
          </IconButton>
        </div>
      </div>

      {renameOpen && (
        <RenameDialog
          initialTitle={clip.title}
          busy={renaming}
          onCancel={() => setRenameOpen(false)}
          onSubmit={(title) => {
            setRenaming(true);
            void rename(clip, title).then((updated) => {
              setRenaming(false);
              if (updated) setRenameOpen(false);
            });
          }}
        />
      )}

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
    </article>
  );
}
