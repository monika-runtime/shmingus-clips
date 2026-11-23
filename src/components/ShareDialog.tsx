import { Check, Copy, ExternalLink, Trash2 } from "lucide-react";
import { useEffect, useId, useRef, useState } from "react";

import { backend, isDesktop } from "../lib/bridge";
import { copyText } from "../lib/clipboard";
import { errorMessage } from "../lib/errors";
import type { Clip, ShareInfo } from "../lib/types";
import { useApp } from "../state/store";
import { Button } from "./ui/Button";
import { ConfirmDialog } from "./ui/ConfirmDialog";
import { Field } from "./ui/Field";
import { Input } from "./ui/Input";
import { Modal } from "./ui/Modal";

interface ShareDialogProps {
  clip: Clip;
  info: ShareInfo;
  onClose: () => void;
}

export function ShareDialog({ clip, info, onClose }: ShareDialogProps) {
  const { pushToast, clearShare } = useApp();
  const titleId = useId();
  const [copied, setCopied] = useState<"link" | "embed" | null>(null);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [removing, setRemoving] = useState(false);
  const copyTimer = useRef<number | null>(null);

  const embedCode = `<iframe src="${info.embedUrl}" width="640" height="360" frameborder="0" allowfullscreen></iframe>`;

  useEffect(
    () => () => {
      if (copyTimer.current !== null) window.clearTimeout(copyTimer.current);
    },
    [],
  );

  async function copy(value: string, what: "link" | "embed") {
    const ok = await copyText(value);
    if (!ok) {
      pushToast(
        "error",
        `Could not copy the ${what === "link" ? "link" : "embed code"} - select it and copy it by hand.`,
      );
      return;
    }
    setCopied(what);
    if (copyTimer.current !== null) window.clearTimeout(copyTimer.current);
    copyTimer.current = window.setTimeout(() => setCopied(null), 1600);
    pushToast(
      "success",
      what === "link" ? "Link copied." : "Embed code copied.",
    );
  }

  async function openInBrowser() {
    try {
      if (isDesktop()) {
        const { openUrl } = await import("@tauri-apps/plugin-opener");
        await openUrl(info.url);
      } else {
        window.open(info.url, "_blank", "noopener,noreferrer");
      }
    } catch (error) {
      pushToast("error", `Could not open the link: ${errorMessage(error)}`);
    }
  }

  async function removeShare() {
    setRemoving(true);
    try {
      await backend.deleteShare(clip.id);
      clearShare(clip.id);
      setConfirmOpen(false);
      pushToast("success", "Share link removed. The clip is still in your library.");
      onClose();
    } catch (error) {
      pushToast("error", `Could not remove the share: ${errorMessage(error)}`);
    } finally {
      setRemoving(false);
    }
  }

  return (
    <Modal onClose={onClose} labelledBy={titleId} className="modal--share">
      <header className="share__head">
        <p className="eyebrow">Share link</p>
        <h2 className="modal__title" id={titleId}>
          {clip.title}
        </h2>
        <p className="modal__subtitle">
          Uploaded with your Shmingus account. Anyone with this link can watch
          the clip until you remove it - no sign-in needed to view.
        </p>
      </header>

      <div className="share__body">
        <Field
          label="Link"
          hint="Paste it into Discord and it becomes a rich embed with the clip's preview."
        >
          <div className="inline-field">
            <Input
              readOnly
              aria-label="Share link"
              value={info.url}
              onFocus={(event) => event.currentTarget.select()}
            />
            <Button
              size="sm"
              icon={copied === "link" ? <Check size={15} /> : <Copy size={15} />}
              onClick={() => void copy(info.url, "link")}
            >
              {copied === "link" ? "Copied" : "Copy"}
            </Button>
            <Button
              size="sm"
              variant="secondary"
              icon={<ExternalLink size={15} />}
              onClick={() => void openInBrowser()}
            >
              Open in browser
            </Button>
          </div>
        </Field>

        <Field label="Embed code" hint="For a page that allows iframes.">
          <div className="share__embed">
            <code className="share__code">{embedCode}</code>
            <Button
              size="sm"
              icon={copied === "embed" ? <Check size={15} /> : <Copy size={15} />}
              onClick={() => void copy(embedCode, "embed")}
            >
              {copied === "embed" ? "Copied" : "Copy"}
            </Button>
          </div>
        </Field>
      </div>

      <div className="modal__footer share__footer">
        <Button
          variant="ghost"
          icon={<Trash2 size={15} />}
          disabled={removing}
          onClick={() => setConfirmOpen(true)}
        >
          Remove from server
        </Button>
        <Button variant="secondary" onClick={onClose}>
          Close
        </Button>
      </div>

      {confirmOpen && (
        <ConfirmDialog
          title="Remove this share?"
          message="The link stops working immediately. The clip stays in your library and its file is untouched."
          confirmLabel="Remove share"
          busy={removing}
          onCancel={() => setConfirmOpen(false)}
          onConfirm={() => void removeShare()}
        />
      )}
    </Modal>
  );
}
