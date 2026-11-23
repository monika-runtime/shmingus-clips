import { useApp } from "../state/store";
import { ShareDialog } from "./ShareDialog";

/** The share dialog is shared by the player and the library cards. */
export function ShareDialogHost() {
  const { shareDialog, closeShareDialog } = useApp();
  if (!shareDialog) return null;

  return (
    <ShareDialog
      key={`${shareDialog.clip.id}:${shareDialog.info.id}`}
      clip={shareDialog.clip}
      info={shareDialog.info}
      onClose={closeShareDialog}
    />
  );
}
