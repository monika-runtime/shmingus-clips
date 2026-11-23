import { RenameDialogHost } from "./components/RenameDialogHost";
import { ShareDialogHost } from "./components/ShareDialogHost";
import { Shell } from "./components/Shell";
import { DashboardScreen } from "./screens/DashboardScreen";
import { LibraryScreen } from "./screens/LibraryScreen";
import { PlayerModal } from "./screens/PlayerModal";
import { SettingsScreen } from "./screens/SettingsScreen";
import { ContextMenuProvider } from "./state/contextMenu";
import { AppProvider, useApp } from "./state/store";

function Screens() {
  const { route } = useApp();
  switch (route) {
    case "library":
      return <LibraryScreen />;
    case "settings":
      return <SettingsScreen />;
    default:
      return <DashboardScreen />;
  }
}

export default function App() {
  return (
    <AppProvider>
      <ContextMenuProvider>
        <Shell>
          <Screens />
        </Shell>
        <PlayerModal />
        <RenameDialogHost />
        <ShareDialogHost />
      </ContextMenuProvider>
    </AppProvider>
  );
}
