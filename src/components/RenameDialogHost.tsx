import { useState } from "react";

import { useApp } from "../state/store";
import { useClipActions } from "../state/useClipActions";
import { RenameDialog } from "./ui/RenameDialog";

/**
 * Rename dialog opened from a toast action (the “Rename” button on the
 * post-save toast), so naming a clip is one click after saving it.
 */
export function RenameDialogHost() {
  const { renameTarget, cancelRename } = useApp();
  const { rename } = useClipActions();
  const [busy, setBusy] = useState(false);

  if (!renameTarget) return null;

  return (
    <RenameDialog
      initialTitle={renameTarget.title}
      busy={busy}
      onCancel={cancelRename}
      onSubmit={(title) => {
        setBusy(true);
        void rename(renameTarget, title).then((updated) => {
          setBusy(false);
          if (updated) cancelRename();
        });
      }}
    />
  );
}
