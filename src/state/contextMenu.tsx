import {
  Activity,
  FolderOpen,
  RefreshCw,
  Settings2,
  type LucideIcon,
} from "lucide-react";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";

import { ContextMenu } from "../components/ui/ContextMenu";
import { backend } from "../lib/bridge";
import { errorMessage } from "../lib/errors";
import { useApp } from "./store";

export interface MenuActionItem {
  label: string;
  icon?: LucideIcon;
  /** Display only - no accelerator is bound to the menu. */
  shortcut?: string;
  disabled?: boolean;
  danger?: boolean;
  onSelect: () => void;
}

export interface MenuSeparator {
  separator: true;
}

export interface MenuHeading {
  /** Small, non-interactive section label above a group of actions. */
  heading: string;
}

export type MenuItem = MenuActionItem | MenuSeparator | MenuHeading;

export function isMenuSeparator(item: MenuItem): item is MenuSeparator {
  return "separator" in item;
}

export function isMenuHeading(item: MenuItem): item is MenuHeading {
  return "heading" in item;
}

/** The parts of a mouse event the menu needs, so a React synthetic event fits. */
export interface ContextMenuEventLike {
  clientX: number;
  clientY: number;
  preventDefault: () => void;
  nativeEvent?: Event;
}

interface MenuState {
  x: number;
  y: number;
  items: MenuItem[];
  /** Forces a remount, so a menu reopened at the same spot refocuses. */
  id: number;
}

interface ContextMenuStore {
  openContextMenu: (event: ContextMenuEventLike, items: MenuItem[]) => void;
  closeContextMenu: () => void;
  /** The app-wide fallback menu, for surfaces that have no menu of their own. */
  defaultMenuItems: MenuItem[];
}

const ContextMenuContext = createContext<ContextMenuStore | null>(null);

/**
 * Native events already claimed by a component's own menu. The global listener
 * sits in the capture phase, so it reaches the event before React's bubble
 * handlers; it therefore opens the fallback on the next task, by which time a
 * specific handler has had the chance to call openContextMenu and claim it.
 */
const claimedEvents = new WeakSet<Event>();

const NATIVE_MENU_TARGET = 'input, textarea, [contenteditable="true"]';

/** Text fields keep WebKit's own editing menu (cut/copy/paste). */
export function isNativeContextTarget(
  target: EventTarget | null,
): boolean {
  if (!(target instanceof Element)) return false;
  return target.closest(NATIVE_MENU_TARGET) !== null;
}

export function ContextMenuProvider({ children }: { children: ReactNode }) {
  const { pushToast, refreshClips, openSettings } = useApp();
  const [menu, setMenu] = useState<MenuState | null>(null);
  const sequence = useRef(0);

  const openClipsFolder = useCallback(() => {
    void backend.openClipsFolder().catch((error: unknown) => {
      pushToast("error", errorMessage(error));
    });
  }, [pushToast]);

  const defaultMenuItems = useMemo<MenuItem[]>(
    () => [
      { label: "Open clips folder", icon: FolderOpen, onSelect: openClipsFolder },
      {
        label: "Refresh library",
        icon: RefreshCw,
        onSelect: () => void refreshClips(),
      },
      { separator: true },
      {
        label: "Settings → Capture",
        icon: Settings2,
        onSelect: () => openSettings("capture"),
      },
      {
        label: "Settings → Diagnostics",
        icon: Activity,
        onSelect: () => openSettings("diagnostics"),
      },
    ],
    [openClipsFolder, openSettings, refreshClips],
  );

  const closeContextMenu = useCallback(() => setMenu(null), []);

  const openContextMenu = useCallback(
    (event: ContextMenuEventLike, items: MenuItem[]) => {
      event.preventDefault();
      if (event.nativeEvent) claimedEvents.add(event.nativeEvent);
      setMenu({
        x: event.clientX,
        y: event.clientY,
        items,
        id: ++sequence.current,
      });
    },
    [],
  );

  // App-wide fallback. Capturing here also keeps WebKit's default navigation
  // menu (back/forward/reload) and its “Save image” entry unreachable on every
  // surface that is not a text field.
  useEffect(() => {
    let active = true;
    const onContextMenu = (event: MouseEvent) => {
      if (isNativeContextTarget(event.target)) return;
      // Never let WebKit's own menu through, even over the menu itself.
      event.preventDefault();
      if (
        event.target instanceof Element &&
        event.target.closest(".context-menu")
      ) {
        return;
      }
      window.setTimeout(() => {
        if (!active || claimedEvents.has(event)) return;
        setMenu({
          x: event.clientX,
          y: event.clientY,
          items: defaultMenuItems,
          id: ++sequence.current,
        });
      }, 0);
    };

    document.addEventListener("contextmenu", onContextMenu, true);
    return () => {
      active = false;
      document.removeEventListener("contextmenu", onContextMenu, true);
    };
  }, [defaultMenuItems]);

  const value = useMemo<ContextMenuStore>(
    () => ({
      openContextMenu,
      closeContextMenu,
      defaultMenuItems,
    }),
    [openContextMenu, closeContextMenu, defaultMenuItems],
  );

  return (
    <ContextMenuContext.Provider value={value}>
      {children}
      {menu && (
        <ContextMenu
          key={menu.id}
          x={menu.x}
          y={menu.y}
          items={menu.items}
          onClose={closeContextMenu}
        />
      )}
    </ContextMenuContext.Provider>
  );
}

export function useContextMenu(): ContextMenuStore {
  const store = useContext(ContextMenuContext);
  if (!store) {
    throw new Error("useContextMenu must be used inside ContextMenuProvider");
  }
  return store;
}
