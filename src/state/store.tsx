import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";

import {
  onClipSaved,
  onGameDetected,
  onLibraryChanged,
  onRecorderStatus,
  onToast,
} from "../lib/api";
import { backend, isDesktop } from "../lib/bridge";
import { DEFAULT_RECORDER_STATUS, DEFAULT_SETTINGS } from "../lib/defaults";
import { errorMessage } from "../lib/errors";
import { checkForUpdate, type UpdateInfo } from "../lib/updates";
import {
  previewSettingsParam,
  previewShareInfo,
  previewShareParam,
} from "../lib/preview";
import type {
  Clip,
  EnvironmentReport,
  GameInfo,
  RecorderStatus,
  SavedMonitor,
  Settings,
  ShareInfo,
  Toast,} from "../lib/types";

export type Route = "dashboard" | "library" | "settings";
export type SettingsSection =
  | "capture"
  | "audio"
  | "hotkeys"
  | "storage"
  | "sharing"
  | "app"
  | "diagnostics";

export interface ShareDialogState {
  clip: Clip;
  info: ShareInfo;
}

export interface ToastAction {
  label: string;
  run: () => void;
}

export interface ToastItem extends Toast {
  id: number;
  /** Optional one-click follow-up (e.g. “Show folder” after an export). */
  action?: ToastAction;
}

interface Busy {
  start: boolean;
  stop: boolean;
  save: boolean;
  mic: boolean;
  monitor: boolean;
}

interface Store {
  mode: "desktop" | "preview";
  /** Loopback HTTP base for clip playback ("" until the backend answers). */
  mediaBase: string;

  route: Route;
  navigate: (route: Route) => void;
  settingsSection: SettingsSection;
  setSettingsSection: (section: SettingsSection) => void;
  openSettings: (section: SettingsSection) => void;

  settings: Settings | null;
  settingsError: string | null;
  savingSettings: boolean;
  applySettings: (next: Settings) => Promise<Settings>;

  status: RecorderStatus;
  statusError: string | null;
  busy: Busy;
  startBuffer: () => Promise<void>;
  stopBuffer: () => Promise<void>;
  saveClipNow: () => Promise<Clip | null>;
  toggleMic: () => Promise<void>;
  chooseMonitor: () => Promise<SavedMonitor | null>;

  game: GameInfo | null;

  environment: EnvironmentReport | null;
  environmentError: string | null;
  environmentBusy: boolean;
  refreshEnvironment: () => Promise<void>;
  setupDismissed: boolean;
  dismissSetup: () => void;

  clips: Clip[];
  clipsLoading: boolean;
  clipsError: string | null;
  refreshClips: (silent?: boolean) => Promise<void>;
  addClip: (clip: Clip) => void;
  patchClip: (clip: Clip) => void;
  removeClip: (id: string) => void;

  /** Shares known to the server, keyed by clip id. */
  shares: Record<string, ShareInfo>;
  refreshShares: () => Promise<void>;
  setShare: (clipId: string, info: ShareInfo) => void;
  clearShare: (clipId: string) => void;

  shareDialog: ShareDialogState | null;
  openShareDialog: (clip: Clip, info: ShareInfo) => void;
  closeShareDialog: () => void;

  playerClip: Clip | null;
  openPlayer: (clip: Clip) => void;
  closePlayer: () => void;

  onboardingDismissed: boolean;
  dismissOnboarding: () => void;

  renameTarget: Clip | null;
  requestRename: (clip: Clip) => void;
  cancelRename: () => void;

  /** A newer release found at launch, until the person dismisses it. */
  update: UpdateInfo | null;
  dismissUpdate: () => void;

  toasts: ToastItem[];
  pushToast: (
    level: Toast["level"],
    message: string,
    action?: ToastAction,
  ) => void;
  dismissToast: (id: number) => void;
}

const AppContext = createContext<Store | null>(null);

function readRoute(): Route {
  const hash = window.location.hash.replace(/^#\/?/, "");
  if (hash === "library" || hash === "settings") return hash;
  return "dashboard";
}

const SETUP_KEY = "clipforge.setup-dismissed";
const UPDATE_DISMISSED_KEY = "clipforge.update-dismissed";
const ONBOARDING_KEY = "clipforge.onboarding-dismissed";

export function AppProvider({ children }: { children: ReactNode }) {
  const desktop = isDesktop();
  const mode: Store["mode"] = desktop ? "desktop" : "preview";
  const [mediaBase, setMediaBase] = useState("");

  const [route, setRoute] = useState<Route>(readRoute);
  const [settingsSection, setSettingsSection] =
    useState<SettingsSection>(() =>
      !isDesktop() && previewSettingsParam() === "sharing"
        ? "sharing"
        : "capture",
    );

  const [settings, setSettings] = useState<Settings | null>(null);
  const [settingsError, setSettingsError] = useState<string | null>(null);
  const [savingSettings, setSavingSettings] = useState(false);

  const [status, setStatus] = useState<RecorderStatus>(DEFAULT_RECORDER_STATUS);
  const [statusError, setStatusError] = useState<string | null>(null);
  const [busy, setBusy] = useState<Busy>({
    start: false,
    stop: false,
    save: false,
    mic: false,
    monitor: false,
  });

  const [game, setGame] = useState<GameInfo | null>(null);

  const [environment, setEnvironment] = useState<EnvironmentReport | null>(null);
  const [environmentError, setEnvironmentError] = useState<string | null>(null);
  const [environmentBusy, setEnvironmentBusy] = useState(false);
  const [setupDismissed, setSetupDismissed] = useState<boolean>(() => {
    try {
      return window.sessionStorage.getItem(SETUP_KEY) === "1";
    } catch {
      return false;
    }
  });

  const [onboardingDismissed, setOnboardingDismissed] = useState<boolean>(() => {
    try {
      return window.localStorage.getItem(ONBOARDING_KEY) === "1";
    } catch {
      return false;
    }
  });

  const [clips, setClips] = useState<Clip[]>([]);
  const [clipsLoading, setClipsLoading] = useState(true);
  const [clipsError, setClipsError] = useState<string | null>(null);

  const [shares, setShares] = useState<Record<string, ShareInfo>>({});
  const [shareDialog, setShareDialog] = useState<ShareDialogState | null>(null);
  const previewShareOpened = useRef(false);

  const [playerClip, setPlayerClip] = useState<Clip | null>(null);
  const [renameTarget, setRenameTarget] = useState<Clip | null>(null);

  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const toastSeq = useRef(0);
  const settingsRef = useRef<Settings | null>(null);

  useEffect(() => {
    settingsRef.current = settings;
  }, [settings]);

  useEffect(() => {
    if (!desktop) return;
    let active = true;
    backend
      .getMediaBase()
      .then((base) => {
        if (active) setMediaBase(base);
      })
      .catch(() => {
        // Playback falls back to the asset protocol; the player will say so.
      });
    return () => {
      active = false;
    };
  }, [desktop]);

  const navigate = useCallback((next: Route) => {
    setRoute(next);
    const target = `#/${next}`;
    if (window.location.hash !== target) {
      window.history.replaceState(null, "", target);
    }
  }, []);

  const openSettings = useCallback(
    (section: SettingsSection) => {
      setSettingsSection(section);
      navigate("settings");
    },
    [navigate],
  );

  const pushToast = useCallback(
    (level: Toast["level"], message: string, action?: ToastAction) => {
      const id = ++toastSeq.current;
      setToasts((list) => [...list.slice(-3), { id, level, message, action }]);
      window.setTimeout(() => {
        setToasts((list) => list.filter((toast) => toast.id !== id));
      }, 4600);
    },
    [],
  );

  const dismissToast = useCallback((id: number) => {
    setToasts((list) => list.filter((toast) => toast.id !== id));
  }, []);

  const dismissUpdate = useCallback(() => {
    setUpdate((current) => {
      if (current) {
        try {
          window.localStorage.setItem(UPDATE_DISMISSED_KEY, current.version);
        } catch {
          // Nothing to remember with; the banner still closes for this session.
        }
      }
      return null;
    });
  }, []);

  // Look for a newer release once the window has settled. Found versions show a
  // banner that stays until dismissed; nothing installs on its own.
  useEffect(() => {
    if (!desktop) return;
    let active = true;
    const timer = window.setTimeout(() => {
      void checkForUpdate()
        .then((info) => {
          if (!active || !info) return;
          let dismissed = "";
          try {
            dismissed = window.localStorage.getItem(UPDATE_DISMISSED_KEY) ?? "";
          } catch {
            // Storage unavailable: show the banner rather than hide the update.
          }
          if (dismissed !== info.version) setUpdate(info);
        })
        .catch(() => {
          // Offline, or the release feed refused: the Updates panel can retry.
        });
    }, 6000);
    return () => {
      active = false;
      window.clearTimeout(timer);
    };
  }, [desktop]);

  const dismissSetup = useCallback(() => {
    setSetupDismissed(true);
    try {
      window.sessionStorage.setItem(SETUP_KEY, "1");
    } catch {
      // Session storage can be unavailable; the banner still dismisses.
    }
  }, []);

  const dismissOnboarding = useCallback(() => {
    setOnboardingDismissed(true);
    try {
      window.localStorage.setItem(ONBOARDING_KEY, "1");
    } catch {
      // Local storage can be unavailable; the card still dismisses for now.
    }
  }, []);

  const requestRename = useCallback((clip: Clip) => setRenameTarget(clip), []);
  const cancelRename = useCallback(() => setRenameTarget(null), []);

  const refreshClips = useCallback(async (silent = false) => {
    if (!silent) setClipsLoading(true);
    try {
      const list = await backend.listClips();
      setClips(list);
      setClipsError(null);
    } catch (error) {
      setClipsError(errorMessage(error));
    } finally {
      if (!silent) setClipsLoading(false);
    }
  }, []);

  const refreshShares = useCallback(async () => {
    try {
      const list = await backend.listShares();
      setShares(list ?? {});
    } catch {
      // Sharing can be off, or the server unreachable. Keep what we last knew
      // rather than stripping every badge because one refresh failed.
    }
  }, []);

  const setShare = useCallback((clipId: string, info: ShareInfo) => {
    setShares((current) => ({ ...current, [clipId]: info }));
  }, []);

  const clearShare = useCallback((clipId: string) => {
    setShares((current) => {
      if (!(clipId in current)) return current;
      const next = { ...current };
      delete next[clipId];
      return next;
    });
  }, []);

  const openShareDialog = useCallback((clip: Clip, info: ShareInfo) => {
    setShareDialog({ clip, info });
  }, []);

  const closeShareDialog = useCallback(() => setShareDialog(null), []);

  const refreshEnvironment = useCallback(async () => {
    setEnvironmentBusy(true);
    try {
      const report = await backend.checkEnvironment();
      setEnvironment(report);
      setEnvironmentError(null);
    } catch (error) {
      setEnvironmentError(errorMessage(error));
    } finally {
      setEnvironmentBusy(false);
    }
  }, []);

  const addClip = useCallback((clip: Clip) => {
    setClips((list) => [clip, ...list.filter((entry) => entry.id !== clip.id)]);
  }, []);

  const patchClip = useCallback((clip: Clip) => {
    setClips((list) =>
      list.map((entry) => (entry.id === clip.id ? clip : entry)),
    );
    setPlayerClip((current) =>
      current && current.id === clip.id ? clip : current,
    );
  }, []);

  const removeClip = useCallback((id: string) => {
    setClips((list) => list.filter((entry) => entry.id !== id));
    setPlayerClip((current) => (current && current.id === id ? null : current));
  }, []);

  const openPlayer = useCallback((clip: Clip) => setPlayerClip(clip), []);
  const closePlayer = useCallback(() => setPlayerClip(null), []);

  const applySettings = useCallback(async (next: Settings) => {
    const previous = settingsRef.current;
    setSettings(next);
    setSavingSettings(true);
    try {
      const saved = await backend.saveSettings(next);
      setSettings(saved);
      setSettingsError(null);
      // A new server URL changes which shares are visible in the library.
      void refreshShares();
      return saved;
    } catch (error) {
      setSettings(previous ?? DEFAULT_SETTINGS);
      throw error;
    } finally {
      setSavingSettings(false);
    }
  }, [refreshShares]);

  const startBuffer = useCallback(async () => {
    setBusy((current) => ({ ...current, start: true }));
    try {
      const next = await backend.startBuffer();
      setStatus(next);
      pushToast("success", "Replay buffer started.");
    } catch (error) {
      const message = errorMessage(error);
      setStatus((current) => ({ ...current, state: "error", error: message }));
      pushToast("error", message);
    } finally {
      setBusy((current) => ({ ...current, start: false }));
    }
  }, [pushToast]);

  const stopBuffer = useCallback(async () => {
    setBusy((current) => ({ ...current, stop: true }));
    try {
      const next = await backend.stopBuffer();
      setStatus(next);
      pushToast("info", "Replay buffer stopped.");
    } catch (error) {
      pushToast("error", errorMessage(error));
    } finally {
      setBusy((current) => ({ ...current, stop: false }));
    }
  }, [pushToast]);

  const saveClipNow = useCallback(async (): Promise<Clip | null> => {
    setBusy((current) => ({ ...current, save: true }));
    try {
      const clip = await backend.saveClip();
      addClip(clip);
      // Saves through a hotkey or the tray raise their own backend toast; this
      // one only covers the in-app button, and offers naming straight away.
      pushToast("success", `Saved - ${clip.title}`, {
        label: "Rename",
        run: () => requestRename(clip),
      });
      void refreshClips(true);
      return clip;
    } catch (error) {
      pushToast("error", errorMessage(error));
      return null;
    } finally {
      setBusy((current) => ({ ...current, save: false }));
    }
  }, [addClip, pushToast, refreshClips, requestRename]);

  const toggleMic = useCallback(async () => {
    setBusy((current) => ({ ...current, mic: true }));
    try {
      const next = await backend.toggleMic();
      setSettings(next);
      setStatus((current) => ({
        ...current,
        micActive: next.audio.micEnabled,
      }));
      pushToast(
        "info",
        next.audio.micEnabled ? "Microphone on." : "Microphone muted.",
      );
    } catch (error) {
      pushToast("error", errorMessage(error));
    } finally {
      setBusy((current) => ({ ...current, mic: false }));
    }
  }, [pushToast]);

  const chooseMonitor = useCallback(async (): Promise<SavedMonitor | null> => {
    setBusy((current) => ({ ...current, monitor: true }));
    try {
      const monitor = await backend.chooseMonitor();
      setSettings((current) =>
        current ? { ...current, captureMonitor: monitor } : current,
      );
      setStatus((current) => ({ ...current, captureLabel: monitor.name }));
      pushToast("success", `Now capturing ${monitor.name}.`);
      return monitor;
    } catch (error) {
      const message = errorMessage(error);
      if (/cancel/i.test(message)) {
        // Dismissing the system picker is not a failure: keep capturing what
        // we already capture and say so, instead of silently doing nothing.
        const current = settingsRef.current?.captureMonitor?.name;
        pushToast(
          "info",
          current
            ? `Screen selection cancelled - still capturing ${current}.`
            : "Screen selection cancelled - capture was left unchanged.",
        );
      } else {
        pushToast("error", `Could not change the screen: ${message}`);
      }
      return null;
    } finally {
      setBusy((current) => ({ ...current, monitor: false }));
    }
  }, [pushToast]);

  // Initial load. Each call fails on its own: a missing backend only takes out
  // the part of the interface that depends on it.
  useEffect(() => {
    let active = true;
    void backend
      .getSettings()
      .then((value) => {
        if (active) setSettings(value);
      })
      .catch((error: unknown) => {
        if (!active) return;
        setSettings(DEFAULT_SETTINGS);
        setSettingsError(errorMessage(error));
      });
    void backend
      .getRecorderStatus()
      .then((value) => {
        if (active) setStatus(value);
      })
      .catch((error: unknown) => {
        if (active) setStatusError(errorMessage(error));
      });
    void backend
      .getDetectedGame()
      .then((value) => {
        if (active) setGame(value);
      })
      .catch(() => {
        // Detection is optional; the dashboard simply shows no game badge.
      });
    void refreshClips();
    void refreshShares();
    void refreshEnvironment();
    return () => {
      active = false;
    };
  }, [refreshClips, refreshShares, refreshEnvironment]);

  // Browser preview only: `?share=<clip id>` opens the share dialog with a
  // sample ShareInfo so the dialog can be reviewed without a share server.
  useEffect(() => {
    if (desktop || previewShareOpened.current) return;
    const wanted = previewShareParam();
    if (!wanted) return;
    const clip = clips.find((entry) => entry.id === wanted);
    if (!clip) return;
    previewShareOpened.current = true;
    const info = previewShareInfo(clip);
    setShares((current) => ({ ...current, [clip.id]: info }));
    setShareDialog({ clip, info });
  }, [desktop, clips]);

  // Backend events. Only the desktop build can listen.
  useEffect(() => {
    if (!desktop) return;
    let active = true;
    const unlisteners: UnlistenFn[] = [];
    const register = (pending: Promise<UnlistenFn>) => {
      pending
        .then((unlisten) => {
          if (active) unlisteners.push(unlisten);
          else unlisten();
        })
        .catch(() => {
          // The runtime may not expose this event yet; polling still works.
        });
    };

    register(onRecorderStatus((next) => setStatus(next)));
    register(
      onClipSaved((clip) => {
        // Hotkey and tray saves already raise a backend toast; adding a second
        // one here repeated the title twice. The event only updates the list.
        addClip(clip);
      }),
    );
    register(onGameDetected((next) => setGame(next)));
    register(
      onLibraryChanged(() => {
        void refreshClips(true);
        void refreshShares();
      }),
    );
    register(onToast((toast) => pushToast(toast.level, toast.message)));

    return () => {
      active = false;
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, [desktop, addClip, pushToast, refreshClips, refreshShares]);

  // The preview backend has no event loop; animate the ring so the design
  // behaves like a live buffer.
  useEffect(() => {
    if (desktop || status.state !== "buffering") return;
    const timer = window.setInterval(() => {
      setStatus((current) =>
        current.state === "buffering"
          ? {
              ...current,
              bufferedSeconds: Math.min(
                current.bufferTargetSeconds,
                current.bufferedSeconds + 1,
              ),
            }
          : current,
      );
    }, 1000);
    return () => window.clearInterval(timer);
  }, [desktop, status.state]);

  const value: Store = {
    mode,
    mediaBase,
    route,
    navigate,
    settingsSection,
    setSettingsSection,
    openSettings,
    settings,
    settingsError,
    savingSettings,
    applySettings,
    status,
    statusError,
    busy,
    startBuffer,
    stopBuffer,
    saveClipNow,
    toggleMic,
    chooseMonitor,
    game,
    environment,
    environmentError,
    environmentBusy,
    refreshEnvironment,
    setupDismissed,
    dismissSetup,
    onboardingDismissed,
    dismissOnboarding,
    clips,
    clipsLoading,
    clipsError,
    refreshClips,
    addClip,
    patchClip,
    removeClip,
    shares,
    refreshShares,
    setShare,
    clearShare,
    shareDialog,
    openShareDialog,
    closeShareDialog,
    playerClip,
    openPlayer,
    closePlayer,
    renameTarget,
    requestRename,
    cancelRename,
    update,
    dismissUpdate,
    toasts,
    pushToast,
    dismissToast,
  };

  return <AppContext.Provider value={value}>{children}</AppContext.Provider>;
}

export function useApp(): Store {
  const store = useContext(AppContext);
  if (!store) throw new Error("useApp must be used inside AppProvider");
  return store;
}
