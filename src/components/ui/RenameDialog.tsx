import { useId, useState } from "react";

import { Button } from "./Button";
import { Input } from "./Input";
import { Modal } from "./Modal";
import { Spinner } from "./Spinner";

interface RenameDialogProps {
  initialTitle: string;
  busy?: boolean;
  onSubmit: (title: string) => void;
  onCancel: () => void;
}

export function RenameDialog({
  initialTitle,
  busy,
  onSubmit,
  onCancel,
}: RenameDialogProps) {
  const titleId = useId();
  const [value, setValue] = useState(initialTitle);
  const trimmed = value.trim();

  return (
    <Modal onClose={onCancel} labelledBy={titleId} className="modal--rename">
      <form
        onSubmit={(event) => {
          event.preventDefault();
          if (!trimmed || busy) return;
          onSubmit(trimmed);
        }}
      >
        <h2 className="modal__title" id={titleId}>
          Rename clip
        </h2>
        <p className="modal__subtitle">
          The file is renamed on disk; the clip keeps its place in the library.
        </p>
        <Input
          value={value}
          maxLength={80}
          autoFocus
          aria-label="Clip title"
          onChange={(event) => setValue(event.target.value)}
        />
        <div className="modal__footer">
          <Button variant="ghost" onClick={onCancel} disabled={busy}>
            Cancel
          </Button>
          <Button
            type="submit"
            variant="primary"
            disabled={!trimmed || busy}
            icon={busy ? <Spinner /> : undefined}
          >
            {busy ? "Renaming…" : "Rename"}
          </Button>
        </div>
      </form>
    </Modal>
  );
}
