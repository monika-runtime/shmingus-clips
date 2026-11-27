import { AlertTriangle } from "lucide-react";
import { useId } from "react";

import { Button } from "./Button";
import { Modal } from "./Modal";
import { Spinner } from "./Spinner";

interface ConfirmDialogProps {
  title: string;
  message: string;
  confirmLabel?: string;
  busy?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

export function ConfirmDialog({
  title,
  message,
  confirmLabel = "Delete",
  busy,
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  const titleId = useId();
  return (
    <Modal
      onClose={onCancel}
      labelledBy={titleId}
      className="modal--confirm"
      lockScroll={false}
    >
      <div className="confirm">
        <span className="confirm__icon" aria-hidden="true">
          <AlertTriangle size={20} />
        </span>
        <div className="confirm__text">
          <h2 className="modal__title" id={titleId}>
            {title}
          </h2>
          <p className="modal__subtitle">{message}</p>
        </div>
      </div>
      <div className="modal__footer">
        <Button variant="ghost" onClick={onCancel} disabled={busy}>
          Cancel
        </Button>
        <Button variant="danger" onClick={onConfirm} disabled={busy} icon={busy ? <Spinner /> : undefined}>
          {busy ? "Working…" : confirmLabel}
        </Button>
      </div>
    </Modal>
  );
}
