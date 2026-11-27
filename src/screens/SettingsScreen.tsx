import {
  Activity,
  AlertTriangle,
  Check,
  CheckCircle2,
  Download,
  FolderOpen,
  Keyboard,
  LogIn,
  LogOut,
  Mic,
  Monitor,
  RefreshCw,
  Share2,
  ShieldCheck,
  SlidersHorizontal,
  Trash2,
  Volume2,
  type LucideIcon,
} from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";

import { Alert } from "../components/ui/Alert";
import { Badge, Chip } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { ConfirmDialog } from "../components/ui/ConfirmDialog";
import { Field } from "../components/ui/Field";
import { Hotkey } from "../components/ui/Hotkey";
import { Input } from "../components/ui/Input";
import { PageHeader } from "../components/ui/PageHeader";
import { Segmented } from "../components/ui/Segmented";
import { Select } from "../components/ui/Select";
import { Slider } from "../components/ui/Slider";
import { Spinner } from "../components/ui/Spinner";
import { Switch } from "../components/ui/Switch";
import { backend, chooseDirectory, isDesktop } from "../lib/bridge";
import { copyText } from "../lib/clipboard";
import { cx } from "../lib/cx";
import { APP_VERSION, DEFAULT_HOTKEYS } from "../lib/defaults";
import { errorMessage } from "../lib/errors";
import { formatBytes, formatDateTime, pluralize, signature } from "../lib/format";
import { previewAuthSample } from "../lib/preview";
import { checkForUpdate, downloadAndInstall, type UpdateInfo } from "../lib/updates";
import type { MicLevel, ServerShare } from "../lib/types";
import {
  applyTheme,
  storedTheme,
  THEMES,
  type AppTheme,
} from "../lib/themes";
import type {
  AudioDevice,
  AuthStatus,
  EncoderKind,
  SelfTestReport,
  Settings,
  ShareServerStatus,
} from "../lib/types";
import { useApp, type SettingsSection } from "../state/store";

const SECTIONS: Array<{ id: SettingsSection; label: string; icon: LucideIcon }> = [
  { id: "capture", label: "Capture", icon: Monitor },
  { id: "audio", label: "Audio", icon: Volume2 },
  { id: "hotkeys", label: "Hotkeys", icon: Keyboard },
  { id: "storage", label: "Storage", icon: FolderOpen },
  { id: "sharing", label: "Sharing", icon: Share2 },
  { id: "app", label: "App", icon: SlidersHorizontal },
  { id: "updates", label: "Updates", icon: Download },
  { id: "diagnostics", label: "Diagnostics", icon: Activity },
];

const BUFFER_OPTIONS = [15, 30, 60, 120].map((value) => ({
  value,
  label: `${value}s`,
}));
const CUSTOM_BUFFER = -1;
const BUFFER_CHOICES = [
  ...BUFFER_OPTIONS,
  { value: CUSTOM_BUFFER, label: "Custom" },
];

const FPS_OPTIONS = [30, 60, 120].map((value) => ({
  value,
  label: `${value}`,
}));

const ENCODER_OPTIONS: Array<{ value: EncoderKind; label: string }> = [
  { value: "auto", label: "Auto" },
  { value: "nvenc", label: "NVENC" },
  { value: "x264", label: "x264" },
];

const MAX_WIDTH_OPTIONS = [
  { value: "0", label: "Full resolution" },
  { value: "1280", label: "1280 px wide" },
  { value: "1920", label: "1920 px wide" },
  { value: "2560", label: "2560 px wide" },
  { value: "3840", label: "3840 px wide" },
];

const FIX_HINTS: Record<string, string> = {
  "portal-screencast":
    "Install xdg-desktop-portal and your desktop's backend (KDE: xdg-desktop-portal-kde), then log back in.",
  "portal-shortcuts":
    "Hotkeys will only work while Shmingus Clips has focus until the portal is available.",
  pipewire:
    "Start PipeWire: systemctl --user enable --now pipewire pipewire-pulse.",
  ffmpeg: "Install FFmpeg with your package manager (pacman -S ffmpeg).",
  ffprobe: "FFprobe ships with the ffmpeg package.",
  "gst-pipewiresrc": "Install gst-plugin-pipewire.",
  "gst-mpegtsmux": "Install gst-plugins-bad.",
  "gst-splitmuxsink": "Install gst-plugins-good.",
  "gst-nvh264enc":
    "NVIDIA hardware encoding is unavailable - the x264 fallback will be used.",
  "gst-x264enc": "Install gst-plugins-ugly for the x264 software encoder.",
};

function Panel({
  title,
  description,
  actions,
  children,
}: {
  title: string;
  description?: string;
  actions?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="card settings-panel">
      <header className="settings-panel__head">
        <div className="settings-panel__heading">
          <h2 className="settings-panel__title">{title}</h2>
          {description && (
            <p className="settings-panel__desc">{description}</p>
          )}
        </div>
        {actions && <div className="settings-panel__actions">{actions}</div>}
      </header>
      <div className="settings-panel__body">{children}</div>
    </section>
  );
}

function SettingRow({
  label,
  description,
  children,
}: {
  label: string;
  description: string;
  children: ReactNode;
}) {
  return (
    <div className="setting-row">
      <div className="setting-row__text">
        <p className="setting-row__label">{label}</p>
        <p className="setting-row__desc">{description}</p>
      </div>
      <div className="setting-row__control">{children}</div>
    </div>
  );
}

/** Map a dBFS value onto the meter: -60 dB is empty, 0 dB is full. */
function micLevelPercent(db: number | undefined): number {
  if (db === undefined || !Number.isFinite(db)) return 0;
  return Math.max(0, Math.min(100, ((db + 60) / 60) * 100));
}

export function SettingsScreen() {
  const {
    settings,
    settingsError,
    savingSettings,
    applySettings,
    settingsSection,
    setSettingsSection,
    chooseMonitor,
    busy,
    clips,
    shares,
    clearShare,
    environment,
    environmentError,
    environmentBusy,
    refreshEnvironment,
    pushToast,
  } = useApp();

  const desktop = isDesktop();

  const [draft, setDraft] = useState<Settings | null>(null);
  const [theme, setTheme] = useState<AppTheme>(() => storedTheme());
  const [saveError, setSaveError] = useState<string | null>(null);
  const [devices, setDevices] = useState<AudioDevice[]>([]);
  const [devicesError, setDevicesError] = useState<string | null>(null);
  const [devicesBusy, setDevicesBusy] = useState(true);
  const [folderBusy, setFolderBusy] = useState(false);
  const [selftest, setSelftest] = useState<SelfTestReport | null>(null);
  const [selftestBusy, setSelftestBusy] = useState(false);
  const [selftestError, setSelftestError] = useState<string | null>(null);
  const [shareStatus, setShareStatus] = useState<ShareServerStatus | null>(null);
  const [shareTestBusy, setShareTestBusy] = useState(false);
  const [shareTestError, setShareTestError] = useState<string | null>(null);
  const [auth, setAuth] = useState<AuthStatus | null>(null);
  const [authLoading, setAuthLoading] = useState(false);
  const [authBusy, setAuthBusy] = useState(false);
  const [authError, setAuthError] = useState<string | null>(null);
  const [updateChecking, setUpdateChecking] = useState(false);
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  const [updateError, setUpdateError] = useState<string | null>(null);
  const [updateInstalling, setUpdateInstalling] = useState(false);
  const [updateProgress, setUpdateProgress] = useState<number | null>(null);
  const [updateChecked, setUpdateChecked] = useState(false);
  const [micLevel, setMicLevel] = useState<MicLevel | null>(null);
  const [customBufferOpen, setCustomBufferOpen] = useState(false);
  const [confirmShareId, setConfirmShareId] = useState<string | null>(null);
  const [shareBusyId, setShareBusyId] = useState<string | null>(null);
  const [serverShares, setServerShares] = useState<ServerShare[]>([]);
  const [serverSharesError, setServerSharesError] = useState<string | null>(null);
  const [serverSharesLoading, setServerSharesLoading] = useState(false);
  const [confirmServerShare, setConfirmServerShare] = useState<ServerShare | null>(null);
  const [customBufferText, setCustomBufferText] = useState("");
  const [micLevelChecked, setMicLevelChecked] = useState(false);

  async function checkUpdates() {
    setUpdateChecking(true);
    setUpdateError(null);
    setUpdateChecked(false);
    try {
      const update = await checkForUpdate();
      setUpdateInfo(update);
      setUpdateChecked(true);
    } catch (error) {
      setUpdateInfo(null);
      setUpdateError(errorMessage(error));
    } finally {
      setUpdateChecking(false);
    }
  }

  async function installUpdate() {
    setUpdateInstalling(true);
    setUpdateError(null);
    try {
      await downloadAndInstall(setUpdateProgress);
      pushToast("success", "Update installed. Restarting…");
    } catch (error) {
      setUpdateError(errorMessage(error));
      setUpdateInstalling(false);
      setUpdateProgress(null);
    }
  }

  async function copyShareLink(clipId: string) {
    const info = shares[clipId];
    if (!info) return;
    const ok = await copyText(info.url);
    pushToast(
      ok ? "success" : "error",
      ok ? "Link copied." : "Could not copy the link; select it by hand.",
    );
  }

  async function removeShare(clipId: string) {
    setShareBusyId(clipId);
    try {
      await backend.deleteShare(clipId);
      clearShare(clipId);
      pushToast("success", "Share link removed. The clip is still in your library.");
      void refreshServerShares();
    } catch (error) {
      pushToast("error", `Could not remove the share: ${errorMessage(error)}`);
    } finally {
      setShareBusyId(null);
      setConfirmShareId(null);
    }
  }

  async function refreshServerShares() {
    if (!desktop) return;
    setServerSharesLoading(true);
    try {
      const list = await backend.listServerShares();
      setServerShares(list);
      setServerSharesError(null);
    } catch (error) {
      setServerSharesError(errorMessage(error));
    } finally {
      setServerSharesLoading(false);
    }
  }

  async function removeServerShare(id: string) {
    setShareBusyId(id);
    try {
      await backend.deleteServerShare(id);
      setServerShares((list) => list.filter((share) => share.id !== id));
      pushToast("success", "Share link removed from the server.");
    } catch (error) {
      pushToast("error", `Could not remove the share: ${errorMessage(error)}`);
    } finally {
      setShareBusyId(null);
      setConfirmServerShare(null);
    }
  }

  async function copyUrl(url: string) {
    const ok = await copyText(url);
    pushToast(
      ok ? "success" : "error",
      ok ? "Link copied." : "Could not copy the link; select it by hand.",
    );
  }

  useEffect(() => {
    if (!desktop || settingsSection !== "sharing") return;
    if (auth?.signedIn) {
      void refreshServerShares();
    } else if (auth) {
      setServerShares([]);
      setServerSharesError(null);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [desktop, settingsSection, auth?.signedIn]);

  useEffect(() => {
    setDraft((current) => current ?? settings);
  }, [settings]);

  useEffect(() => {
    let active = true;
    setDevicesBusy(true);
    backend
      .getAudioDevices()
      .then((list) => {
        if (!active) return;
        setDevices(list);
        setDevicesError(null);
      })
      .catch((error: unknown) => {
        if (active) setDevicesError(errorMessage(error));
      })
      .finally(() => {
        if (active) setDevicesBusy(false);
      });
    return () => {
      active = false;
    };
  }, []);

  // Stream microphone levels while the Audio section is open.
  useEffect(() => {
    if (!desktop || settingsSection !== "audio" || !settings?.audio.micEnabled) {
      setMicLevel(null);
      setMicLevelChecked(false);
      return;
    }
    let active = true;
    let unlisten: (() => void) | null = null;
    setMicLevel(null);
    setMicLevelChecked(false);
    void backend.startMicMonitor().catch(() => {});
    const silence = window.setTimeout(() => {
      if (active) setMicLevelChecked(true);
    }, 1500);
    backend
      .onMicLevel((level) => {
        if (!active) return;
        setMicLevel(level);
        setMicLevelChecked(true);
      })
      .then((stop) => {
        if (active) unlisten = stop;
        else stop();
      })
      .catch(() => {
        // Event delivery is unavailable; the meter keeps saying "measuring".
      });
    return () => {
      active = false;
      window.clearTimeout(silence);
      unlisten?.();
      void backend.stopMicMonitor().catch(() => {});
    };
  }, [desktop, settingsSection, settings?.audio.micEnabled, settings?.audio.micDevice]);

  // Ask the backend for the Authentik session whenever Sharing is opened, so
  // the card reflects a sign-in that happened outside this screen.
  useEffect(() => {
    if (settingsSection !== "sharing") return;
    let active = true;
    setAuthLoading(true);
    setAuthError(null);
    backend
      .authStatus()
      .then((status) => {
        if (active) setAuth(status);
      })
      .catch((error: unknown) => {
        if (active) setAuthError(errorMessage(error));
      })
      .finally(() => {
        if (active) setAuthLoading(false);
      });
    return () => {
      active = false;
    };
  }, [settingsSection]);

  if (!draft) {
    return (
      <div className="page">
        <PageHeader
          eyebrow="Settings"
          title="Preferences"
          description="Choose what Shmingus Clips records, where clips go, and how it behaves."
        />
        <div className="card settings-loading">
          <Spinner size={20} />
          <span>Loading settings…</span>
        </div>
      </div>
    );
  }

  const dirty = settings ? signature(draft) !== signature(settings) : false;

  function patch(partial: Partial<Settings>) {
    setDraft((current) => (current ? { ...current, ...partial } : current));
  }

  async function handleSave() {
    if (!draft) return;
    setSaveError(null);
    try {
      const saved = await applySettings(draft);
      setDraft(saved);
      pushToast("success", "Settings saved.");
    } catch (error) {
      const message = errorMessage(error);
      setSaveError(message);
      pushToast("error", `Could not save settings: ${message}`);
    }
  }

  function discard() {
    if (settings) setDraft(settings);
    setSaveError(null);
  }

  async function handleChooseMonitor() {
    const monitor = await chooseMonitor();
    if (monitor) patch({ captureMonitor: monitor });
  }

  async function pickFolder() {
    if (!draft) return;
    setFolderBusy(true);
    try {
      const directory = await chooseDirectory(draft.saveDir || undefined);
      if (directory) patch({ saveDir: directory });
    } catch (error) {
      pushToast("error", errorMessage(error));
    } finally {
      setFolderBusy(false);
    }
  }

  async function openFolder() {
    setFolderBusy(true);
    try {
      await backend.openClipsFolder();
    } catch (error) {
      pushToast("error", errorMessage(error));
    } finally {
      setFolderBusy(false);
    }
  }

  async function runSelfTest() {
    setSelftestBusy(true);
    setSelftestError(null);
    try {
      const report = await backend.runSelftest();
      setSelftest(report);
    } catch (error) {
      setSelftestError(errorMessage(error));
    } finally {
      setSelftestBusy(false);
    }
  }

  async function testShareServer() {
    if (!draft) return;
    setShareTestBusy(true);
    setShareTestError(null);
    setShareStatus(null);
    try {
      // The backend tests the saved settings, so save the draft first when the
      // typed server details are not on disk yet.
      if (dirty) {
        const saved = await applySettings(draft);
        setDraft(saved);
      }
      const status = await backend.testShareServer();
      setShareStatus(status);
    } catch (error) {
      setShareTestError(errorMessage(error));
    } finally {
      setShareTestBusy(false);
    }
  }

  async function signIn() {
    setAuthBusy(true);
    setAuthError(null);
    try {
      // The command opens the system browser and returns once Authentik calls
      // back, so this can wait a minute or longer.
      const status = await backend.authLogin();
      setAuth(status);
      if (status.signedIn) {
        pushToast(
          "success",
          `Signed in as ${status.name ?? status.sub ?? "your Shmingus account"}.`,
        );
        void refreshServerShares();
      } else {
        setAuthError("Sign-in did not finish. Try again.");
      }
    } catch (error) {
      setAuthError(errorMessage(error));
    } finally {
      setAuthBusy(false);
    }
  }

  async function signOut() {
    setAuthBusy(true);
    setAuthError(null);
    try {
      await backend.authLogout();
      // Refresh rather than assuming: the backend may already have dropped the
      // token while the browser session is still open.
      const status = await backend.authStatus().catch(() => null);
      setAuth(
        status ?? {
          signedIn: false,
          name: null,
          email: null,
          sub: null,
          expiresAt: null,
        },
      );
      pushToast("success", "Signed out of your Shmingus account.");
    } catch (error) {
      setAuthError(errorMessage(error));
    } finally {
      setAuthBusy(false);
    }
  }

  const micOptions = [{ value: "", label: "System default" }];
  for (const device of devices) {
    if (device.isMonitor) continue;
    micOptions.push({
      value: device.id,
      label: device.isDefault ? `${device.label} (default)` : device.label,
    });
  }
  if (
    draft.audio.micDevice &&
    !micOptions.some((option) => option.value === draft.audio.micDevice)
  ) {
    micOptions.push({
      value: draft.audio.micDevice,
      label: "Saved microphone (not connected)",
    });
  }

  const maxWidthOptions = MAX_WIDTH_OPTIONS.some(
    (option) => option.value === String(draft.maxWidth),
  )
    ? MAX_WIDTH_OPTIONS
    : [
        { value: String(draft.maxWidth), label: `${draft.maxWidth} px wide` },
        ...MAX_WIDTH_OPTIONS,
      ];

  const diskUsed = clips.reduce((total, clip) => total + clip.sizeBytes, 0);
  const accountLabel = (auth?.name ?? auth?.sub ?? auth?.email ?? "").trim();
  const accountInitials = accountLabel
    ? accountLabel
        .split(/\s+/)
        .slice(0, 2)
        .map((part) => part[0])
        .join("")
        .toUpperCase()
    : "?";
  const failingRequired = environment
    ? environment.checks.filter((check) => !check.ok && check.required).length
    : 0;

  const sharedEntries = Object.entries(shares)
    .map(([clipId, info]) => ({
      clipId,
      info,
      clip: clips.find((clip) => clip.id === clipId),
    }))
    .sort((a, b) => Date.parse(b.info.createdAt) - Date.parse(a.info.createdAt));

  const localShareIds = new Set(
    Object.values(shares).map((share) => share.id),
  );
  const remoteShares = serverShares.filter(
    (share) => !localShareIds.has(share.id),
  );

  return (
    <div className="page">
      <PageHeader
        eyebrow="Settings"
        title="Preferences"
        description="Choose what Shmingus Clips records, where clips go, and how it behaves. Changes take effect right away."
      />

      {settingsError && (
        <Alert tone="warn" title="Settings backend unavailable">
          Showing built-in defaults: {settingsError}
        </Alert>
      )}

      <div className="settings-layout">
        <nav className="settings-nav" aria-label="Settings sections">
          {SECTIONS.map(({ id, label, icon: Icon }) => (
            <button
              key={id}
              type="button"
              className={
                settingsSection === id
                  ? "settings-nav__item is-active"
                  : "settings-nav__item"
              }
              aria-current={settingsSection === id ? "true" : undefined}
              onClick={() => setSettingsSection(id)}
            >
              <Icon size={16} aria-hidden="true" />
              <span>{label}</span>
            </button>
          ))}
        </nav>

        <div className="settings-content">
          {settingsSection === "capture" && (
            <>
              <Panel
                title="What gets recorded"
                description="The screen Shmingus Clips watches, and how much it keeps ready."
              >
                <div className="setting-block">
                  <Field
                    label="Screen to record"
                    hint="Your system asks which screen to share and remembers the choice next time."
                  >
                    <div className="inline-field">
                      <span className="inline-field__value">
                        {draft.captureMonitor?.name ?? "No screen selected"}
                      </span>
                      <Button
                        size="sm"
                        icon={busy.monitor ? <Spinner /> : <Monitor size={15} />}
                        disabled={busy.monitor}
                        onClick={() => void handleChooseMonitor()}
                      >
                        {busy.monitor ? "Choosing…" : "Choose screen"}
                      </Button>
                    </div>
                  </Field>
                </div>

                <div className="setting-block">
                  <Field
                    label="How far back to keep"
                    hint="The last moments stay in memory, so you can save them after something happens."
                  >
                    <Segmented
                      label="How far back to keep"
                      value={
                        customBufferOpen ||
                        !BUFFER_OPTIONS.some(
                          (option) => option.value === draft.bufferSeconds,
                        )
                          ? CUSTOM_BUFFER
                          : draft.bufferSeconds
                      }
                      options={BUFFER_CHOICES}
                      onChange={(value) => {
                        if (value === CUSTOM_BUFFER) {
                          setCustomBufferOpen(true);
                          setCustomBufferText(String(draft.bufferSeconds));
                          return;
                        }
                        setCustomBufferOpen(false);
                        setCustomBufferText("");
                        patch({ bufferSeconds: value });
                      }}
                    />
                    {(customBufferOpen ||
                      !BUFFER_OPTIONS.some(
                        (option) => option.value === draft.bufferSeconds,
                      )) && (
                      <div className="inline-field buffer-custom">
                        <Input
                          type="number"
                          inputMode="numeric"
                          min={5}
                          max={300}
                          step={1}
                          aria-label="Seconds to keep"
                          value={
                            customBufferText || String(draft.bufferSeconds)
                          }
                          onChange={(event) => {
                            const text = event.target.value;
                            setCustomBufferText(text);
                            const seconds = Number(text);
                            if (
                              Number.isInteger(seconds) &&
                              seconds >= 5 &&
                              seconds <= 300
                            ) {
                              patch({ bufferSeconds: seconds });
                            }
                          }}
                          onBlur={() => {
                            const seconds = Math.round(Number(customBufferText));
                            if (!Number.isFinite(seconds)) {
                              setCustomBufferText(String(draft.bufferSeconds));
                              return;
                            }
                            const clamped = Math.min(300, Math.max(5, seconds));
                            setCustomBufferText(String(clamped));
                            patch({ bufferSeconds: clamped });
                          }}
                        />
                        <span className="inline-field__value">
                          seconds (5 to 300)
                        </span>
                      </div>
                    )}
                  </Field>
                </div>

                <details className="settings-advanced">
                  <summary className="settings-advanced__summary">
                    <span className="settings-advanced__title">Advanced</span>
                    <span className="settings-advanced__hint">
                      Video encoder, quality, output size and frame rate
                    </span>
                  </summary>
                  <div className="settings-advanced__body">
                    <div className="settings-grid">
                      <Field label="Frame rate">
                        <Segmented
                          label="Frame rate"
                          value={draft.fps}
                          options={FPS_OPTIONS}
                          onChange={(value) => patch({ fps: value })}
                        />
                      </Field>

                      <Field
                        label="Video encoder"
                        hint="Auto uses your graphics card when it can. NVENC is NVIDIA hardware; x264 encodes on the processor instead."
                      >
                        <Segmented
                          label="Video encoder"
                          value={draft.encoder}
                          options={ENCODER_OPTIONS}
                          onChange={(value) => patch({ encoder: value })}
                        />
                      </Field>

                      <Slider
                        label="Quality (bitrate)"
                        min={5}
                        max={60}
                        step={1}
                        value={draft.bitrateMbps}
                        formatValue={(value) => `${value} Mbps`}
                        hint="Higher is sharper and makes bigger files."
                        onChange={(value) => patch({ bitrateMbps: value })}
                      />

                      <Field
                        label="Output size"
                        hint="Downscale wide screens to keep files small."
                      >
                        <Select
                          label="Output size"
                          value={String(draft.maxWidth)}
                          options={maxWidthOptions}
                          onChange={(value) => patch({ maxWidth: Number(value) })}
                        />
                      </Field>
                    </div>
                  </div>
                </details>
              </Panel>

              <Panel
                title="Game awareness"
                description="Name clips after the game that was running."
              >
                <Switch
                  label="Detect running games"
                  description="Tags new clips with the game name and groups them in the library."
                  icon={<Monitor size={18} />}
                  checked={draft.autoDetectGames}
                  onChange={(next) => patch({ autoDetectGames: next })}
                />
              </Panel>
            </>
          )}

          {settingsSection === "audio" && (
            <Panel
              title="Track mix"
              description="Desktop sound and microphone are captured as separate tracks."
            >
              <Switch
                label="Desktop audio"
                description="Record the sound of the captured screen."
                icon={<Volume2 size={18} />}
                checked={draft.audio.desktopEnabled}
                onChange={(next) =>
                  patch({
                    audio: { ...draft.audio, desktopEnabled: next },
                  })
                }
              />
              <Slider
                label="Desktop volume"
                min={0}
                max={2}
                step={0.05}
                value={draft.audio.desktopVolume}
                formatValue={(value) => `${Math.round(value * 100)}%`}
                disabled={!draft.audio.desktopEnabled}
                onChange={(value) =>
                  patch({ audio: { ...draft.audio, desktopVolume: value } })
                }
              />

              <div className="settings-divider" />

              <Switch
                label="Microphone"
                description="Add your microphone as the second audio track."
                icon={<Mic size={18} />}
                checked={draft.audio.micEnabled}
                onChange={(next) =>
                  patch({ audio: { ...draft.audio, micEnabled: next } })
                }
              />
              <Field
                label="Microphone device"
                hint={
                  devicesBusy
                    ? "Looking for microphones…"
                    : "Leave it on system default unless you use a separate microphone."
                }
              >
                <Select
                  label="Microphone device"
                  value={draft.audio.micDevice ?? ""}
                  options={micOptions}
                  disabled={devicesBusy}
                  onChange={(value) =>
                    patch({
                      audio: {
                        ...draft.audio,
                        micDevice: value === "" ? null : value,
                      },
                    })
                  }
                />
              </Field>
              {devicesError && (
                <Alert tone="warn" title="Audio devices unavailable">
                  {devicesError}
                </Alert>
              )}
              <Slider
                label="Microphone volume"
                min={0}
                max={2}
                step={0.05}
                value={draft.audio.micVolume}
                formatValue={(value) => `${Math.round(value * 100)}%`}
                disabled={!draft.audio.micEnabled}
                onChange={(value) =>
                  patch({ audio: { ...draft.audio, micVolume: value } })
                }
              />
              {draft.audio.micEnabled && (
                <div className="mic-meter">
                  <div className="mic-meter__head">
                    <span className="micro-label">Input level</span>
                    <span className="mic-meter__value">
                      {micLevel
                        ? `${Math.round(micLevel.peakDb)} dB peak`
                        : micLevelChecked
                          ? "no signal"
                          : "measuring…"}
                    </span>
                  </div>
                  <div
                    className="mic-meter__track"
                    role="meter"
                    aria-label="Microphone input level"
                    aria-valuemin={-60}
                    aria-valuemax={0}
                    aria-valuenow={
                      micLevel ? Math.round(micLevel.peakDb) : undefined
                    }
                  >
                    <span
                      className={cx(
                        "mic-meter__fill",
                        micLevel && micLevel.peakDb > -6 && "is-hot",
                        micLevel && micLevel.peakDb < -40 && "is-quiet",
                      )}
                      style={{ width: `${micLevelPercent(micLevel?.peakDb)}%` }}
                    />
                  </div>
                  <p className="mic-meter__hint">
                    {!micLevelChecked
                      ? "Measuring the microphone…"
                      : micLevel === null
                        ? "No microphone signal. Check the system input volume and the device above."
                        : micLevel.peakDb < -40
                          ? "Very quiet. Raise the system input volume, or the microphone volume above."
                          : "Speak normally; the bar should reach the middle."}
                  </p>
                </div>
              )}
              <Switch
                label="Even out the microphone level (experimental)"
                description="At save time, lift a quiet microphone toward the game audio in the mixed track. The separate microphone track stays untouched."
                checked={draft.audio.micAutoGain}
                disabled={!draft.audio.micEnabled}
                onChange={(next) =>
                  patch({ audio: { ...draft.audio, micAutoGain: next } })
                }
              />
            </Panel>
          )}

          {settingsSection === "hotkeys" && (
            <Panel
              title="Global shortcuts"
              description="Work while a game has focus, through the GlobalShortcuts portal."
              actions={
                <Button
                  size="sm"
                  variant="secondary"
                  onClick={() => patch({ hotkeys: { ...DEFAULT_HOTKEYS } })}
                >
                  Reset to defaults
                </Button>
              }
            >
              <SettingRow
                label="Save clip"
                description="Cut the last seconds of the replay buffer into a new clip."
              >
                <Hotkey accelerator={draft.hotkeys.saveClip} />
              </SettingRow>
              <SettingRow
                label="Toggle buffer"
                description="Start or stop the always-on replay buffer."
              >
                <Hotkey accelerator={draft.hotkeys.toggleBuffer} />
              </SettingRow>
              <SettingRow
                label="Toggle microphone"
                description="Mute or unmute your microphone without restarting capture."
              >
                <Hotkey accelerator={draft.hotkeys.toggleMic} />
              </SettingRow>
              <Alert tone="info">
                Some desktops ask for approval the first time Shmingus Clips binds a
                shortcut. Reset restores CTRL+ALT+C, CTRL+ALT+B and CTRL+ALT+M.
              </Alert>
            </Panel>
          )}

          {settingsSection === "storage" && (
            <Panel
              title="Where clips live"
              description="Recordings are plain MP4 files in your own folder."
            >
              <Field label="Clips folder">
                <div className="inline-field">
                  <Input
                    readOnly
                    aria-label="Clips folder"
                    value={draft.saveDir}
                    placeholder="No folder chosen yet"
                  />
                  <Button
                    size="sm"
                    icon={folderBusy ? <Spinner /> : <FolderOpen size={15} />}
                    disabled={folderBusy}
                    onClick={() => void pickFolder()}
                  >
                    Choose folder
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={folderBusy || !settings?.saveDir}
                    onClick={() => void openFolder()}
                  >
                    Open
                  </Button>
                </div>
              </Field>
              <div className="usage">
                <div className="usage__item">
                  <span className="micro-label">Clips</span>
                  <span className="usage__value">{clips.length}</span>
                </div>
                <div className="usage__item">
                  <span className="micro-label">On disk</span>
                  <span className="usage__value">{formatBytes(diskUsed)}</span>
                </div>
                <div className="usage__item">
                  <span className="micro-label">Exports</span>
                  <span className="usage__value">Shmingus Clips/Exports</span>
                </div>
              </div>
              <Alert tone="info">
                New clips are grouped into a folder per game. Exports are copies
                and never move the original.
              </Alert>
            </Panel>
          )}

          {settingsSection === "sharing" && (
            <>
              <Panel
                title="Account"
                description="Uploads use your Shmingus account. Sign in once on this computer; your password is only ever entered in the browser."
              >
                {auth === null && authLoading ? (
                  <div className="account-card account-card--muted" aria-busy="true">
                    <span className="account-card__avatar">
                      <Spinner size={18} />
                    </span>
                    <div className="account-card__body">
                      <p className="account-card__name">Checking your account…</p>
                    </div>
                  </div>
                ) : auth?.signedIn ? (
                  <div className="account-card">
                    <span className="account-card__avatar" aria-hidden="true">
                      {accountInitials}
                    </span>
                    <div className="account-card__body">
                      <p className="account-card__name">
                        {auth.name ?? auth.sub ?? "Signed in"}
                        {previewAuthSample() && (
                          <Badge tone="neutral" className="account-card__flag">
                            Preview sample
                          </Badge>
                        )}
                      </p>
                      {auth.email && (
                        <p className="account-card__email">{auth.email}</p>
                      )}
                      <p className="account-card__meta">
                        Signed in with your Shmingus account. Clip links stay
                        public to anyone who has them.
                      </p>
                      {authError && (
                        <p className="account-card__error" role="alert">
                          {authError}
                        </p>
                      )}
                    </div>
                    <Button
                      size="sm"
                      variant="ghost"
                      icon={authBusy ? <Spinner size={14} /> : <LogOut size={15} />}
                      disabled={authBusy}
                      onClick={() => void signOut()}
                    >
                      {authBusy ? "Signing out…" : "Sign out"}
                    </Button>
                  </div>
                ) : (
                  <div className="account-card account-card--signed-out">
                    <span className="account-card__avatar" aria-hidden="true">
                      <ShieldCheck size={19} />
                    </span>
                    <div className="account-card__body">
                      <p className="account-card__name">Not signed in</p>
                      <p className="account-card__meta">
                        Signing in opens your browser. Shmingus Clips never sees
                        your password.
                      </p>
                      {authBusy && (
                        <p className="account-card__note" role="status">
                          Finish signing in in your browser. This can take a
                          minute.
                        </p>
                      )}
                      {authError && (
                        <p className="account-card__error" role="alert">
                          {authError}
                        </p>
                      )}
                    </div>
                    <Button
                      size="sm"
                      variant="primary"
                      icon={
                        authBusy ? <Spinner size={14} /> : <LogIn size={15} />
                      }
                      disabled={authBusy || authLoading}
                      onClick={() => void signIn()}
                    >
                      {authBusy
                        ? "Waiting for browser…"
                        : "Sign in with your Shmingus account"}
                    </Button>
                  </div>
                )}
              </Panel>

              <Panel
                title="Shared clips"
                description="Copy a link again or take it off the server without opening the clip."
              >
                {sharedEntries.length === 0 && remoteShares.length === 0 ? (
                  <p className="share-list__empty">No clips are shared yet.</p>
                ) : (
                  <ul className="share-list">
                    {sharedEntries.map(({ clipId, info, clip }) => (
                      <li key={clipId} className="share-list__item">
                        <div className="share-list__text">
                          <p className="share-list__title">
                            {clip?.title ?? "Missing clip"}
                          </p>
                          <p className="share-list__meta">
                            {formatDateTime(info.createdAt)} ·{" "}
                            <span className="share-list__url">{info.url}</span>
                          </p>
                        </div>
                        <div className="share-list__actions">
                          <Button
                            size="sm"
                            variant="secondary"
                            onClick={() => void copyShareLink(clipId)}
                          >
                            Copy link
                          </Button>
                          <Button
                            size="sm"
                            variant="ghost"
                            className="share-list__remove"
                            icon={<Trash2 size={14} />}
                            disabled={shareBusyId === clipId}
                            onClick={() => setConfirmShareId(clipId)}
                          >
                            Remove
                          </Button>
                        </div>
                      </li>
                    ))}
                    {remoteShares.map((share) => (
                      <li key={share.id} className="share-list__item">
                        <div className="share-list__text">
                          <p className="share-list__title">{share.title}</p>
                          <p className="share-list__meta">
                            {formatDateTime(share.createdAt)} ·{" "}
                            <span className="share-list__url">{share.url}</span>
                          </p>
                        </div>
                        <div className="share-list__actions">
                          <Button
                            size="sm"
                            variant="secondary"
                            onClick={() => void copyUrl(share.url)}
                          >
                            Copy link
                          </Button>
                          <Button
                            size="sm"
                            variant="ghost"
                            className="share-list__remove"
                            icon={<Trash2 size={14} />}
                            disabled={shareBusyId === share.id}
                            onClick={() => setConfirmServerShare(share)}
                          >
                            Remove
                          </Button>
                        </div>
                      </li>
                    ))}
                  </ul>
                )}
                {serverSharesLoading && (
                  <p className="share-list__empty">Checking your account…</p>
                )}
                {serverSharesError && (
                  <p className="share-list__empty">{serverSharesError}</p>
                )}
              </Panel>

              <Panel
                title="Share server"
                description="Send a clip to your own share server and get a link you can paste into chat."
                actions={
                  <Button
                    size="sm"
                    variant="secondary"
                    icon={shareTestBusy ? <Spinner /> : <RefreshCw size={15} />}
                    disabled={shareTestBusy || !draft.shareBaseUrl.trim()}
                    onClick={() => void testShareServer()}
                  >
                    {shareTestBusy ? "Testing…" : "Test connection"}
                  </Button>
                }
              >
                <Field
                  label="Server URL"
                  hint="The address of your Shmingus Clips share server, for example https://clips.shmingus.net. Leave it empty to keep sharing off."
                >
                  <Input
                    aria-label="Share server URL"
                    value={draft.shareBaseUrl}
                    placeholder="https://clips.shmingus.net"
                    onChange={(event) => {
                      patch({ shareBaseUrl: event.target.value });
                      setShareStatus(null);
                      setShareTestError(null);
                    }}
                  />
                </Field>

                {shareStatus && (
                  <Alert
                    tone={shareStatus.ok ? "success" : "warn"}
                    title={shareStatus.ok ? "Connected" : "Could not connect"}
                  >
                    {shareStatus.detail}
                  </Alert>
                )}
                {shareTestError && (
                  <Alert tone="warn" title="Connection test failed">
                    {shareTestError}
                  </Alert>
                )}
                {!draft.shareBaseUrl.trim() && (
                  <Alert tone="info">
                    Sharing is off until you add a server URL. Clips stay on this
                    computer.
                  </Alert>
                )}
              </Panel>

              <Panel
                title="How sharing works"
                description="A few things to know before a clip leaves this computer."
              >
                <ul className="share-help">
                  <li>
                    Uploads use your Shmingus account.
                    Viewing a link never needs an account - anyone with the link
                    can watch the clip.
                  </li>
                  <li>
                    Clips larger than 10 MB are re-encoded on this computer for
                    the share (up to 720p, under 10 MB) so chat apps can embed
                    them. Your library keeps the full-quality original.
                  </li>
                  <li>
                    Links are permanent until you remove the share. The clip
                    itself stays in your library either way.
                  </li>
                  <li>
                    Discord only shows a rich embed when the link is a public
                    HTTPS address - not localhost and not plain HTTP.
                  </li>
                </ul>
                <p className="share-help__foot">
                  Share a clip from the player, or right-click a clip in the
                  library and choose Share.
                </p>
              </Panel>
            </>
          )}

          {settingsSection === "app" && (
            <>
              <Panel
                title="Theme"
                description="Choose the color theme. Applies immediately and is remembered on this device."
              >
                <div className="theme-picker" role="radiogroup" aria-label="Color theme">
                  {THEMES.map((option) => (
                    <button
                      key={option.id}
                      type="button"
                      role="radio"
                      aria-checked={theme === option.id}
                      className={
                        theme === option.id ? "theme-card is-on" : "theme-card"
                      }
                      onClick={() => {
                        applyTheme(option.id);
                        setTheme(option.id);
                        pushToast("info", `${option.label} theme applied.`);
                      }}
                    >
                      <span className="theme-card__swatch" aria-hidden="true">
                        {option.swatch.map((color) => (
                          <i key={color} style={{ background: color }} />
                        ))}
                      </span>
                      <span className="theme-card__name">{option.label}</span>
                      <span className="theme-card__desc">
                        {option.description}
                      </span>
                    </button>
                  ))}
                </div>
              </Panel>
              <Panel
                title="Behaviour"
                description="How Shmingus Clips starts and stays out of the way."
              >
              <Switch
                label="Start buffer on launch"
                description="Begin buffering as soon as Shmingus Clips opens."
                checked={draft.startBufferOnLaunch}
                onChange={(next) => patch({ startBufferOnLaunch: next })}
              />
              <Switch
                label="Start automatically when a game is detected"
                description="Start the replay buffer when a game launches and stop it when the game exits."
                checked={draft.autoStartOnGame}
                onChange={(next) => patch({ autoStartOnGame: next })}
              />
              <Switch
                label="Save a clip when a game exits"
                description="Keep the last buffered moments when a detected game closes or crashes. A crash looks the same as a quit to the detector."
                checked={draft.saveOnGameExit}
                onChange={(next) => patch({ saveOnGameExit: next })}
              />
              <Switch
                label="Minimize to tray"
                description="Closing the window keeps the buffer running in the tray."
                checked={draft.minimizeToTray}
                onChange={(next) => patch({ minimizeToTray: next })}
              />
              <Switch
                label="Launch at login"
                description="Open Shmingus Clips automatically when you sign in."
                checked={draft.launchAtLogin}
                onChange={(next) => patch({ launchAtLogin: next })}
              />
              <Switch
                label="Notify when a clip is saved"
                description="Show a desktop notification after each capture."
                checked={draft.notifyOnClip}
                onChange={(next) => patch({ notifyOnClip: next })}
              />
              <Switch
                label="Play a sound when a clip is saved"
                description="A short chime, audible over a fullscreen game even when notifications are hidden."
                checked={draft.clipSound}
                onChange={(next) => patch({ clipSound: next })}
              />
              <Switch
                label="Compatibility rendering"
                description="Draw the interface without GPU acceleration. Use this if the window crashes right after opening on NVIDIA Wayland; Shmingus Clips also turns it on by itself after a failed start. Takes effect on the next launch; if it then refuses to open, launch once with --no-software-rendering to turn it back off."
                checked={draft.softwareRendering}
                onChange={(next) => patch({ softwareRendering: next })}
              />
              </Panel>
            </>
          )}

          {settingsSection === "updates" && (
            <>
            <Panel
              title="Updates"
              actions={
                <Button
                  size="sm"
                  variant="secondary"
                  icon={updateChecking ? <Spinner /> : <RefreshCw size={15} />}
                  disabled={!desktop || updateChecking || updateInstalling}
                  onClick={() => void checkUpdates()}
                >
                  {updateChecking ? "Checking…" : "Check for updates"}
                </Button>
              }
            >
              <div className="update-card">
                <div className="update-card__body">
                  <p className="update-card__version">Version {APP_VERSION}</p>
                  <p className="update-card__note">
                    {updateInstalling
                      ? updateProgress === null
                        ? "Downloading and installing…"
                        : `Downloading and installing… ${updateProgress}%`
                      : updateInfo
                        ? `Version ${updateInfo.version} is ready to install.`
                        : updateChecked
                          ? "You are on the latest version."
                          : "Checked at launch and every 12 hours; you can check any time."}
                  </p>
                </div>
                {updateInfo && !updateInstalling && (
                  <Button
                    variant="primary"
                    icon={<Download size={15} />}
                    onClick={() => void installUpdate()}
                  >
                    Install and restart
                  </Button>
                )}
              </div>
              {updateError && (
                <Alert tone="warn" title="Update check failed">
                  {updateError}
                </Alert>
              )}
            </Panel>
            </>
          )}

          {settingsSection === "diagnostics" && (
            <>
              <Panel
                title="Can this computer record?"
                description="Checks the pieces Shmingus Clips needs to record your screen, and says what to fix when one is missing."
                actions={
                  <Button
                    size="sm"
                    variant="secondary"
                    icon={environmentBusy ? <Spinner /> : <RefreshCw size={15} />}
                    disabled={environmentBusy}
                    onClick={() => void refreshEnvironment()}
                  >
                    {environmentBusy ? "Checking…" : "Check again"}
                  </Button>
                }
              >
                {environmentError && (
                  <Alert tone="warn" title="Environment check failed">
                    {environmentError}
                  </Alert>
                )}
                {environment ? (
                  <>
                    <div className="diag-summary">
                      <Badge tone={environment.ok ? "accent" : "warn"} dot>
                        {environment.ok
                          ? "Ready to capture"
                          : "Needs attention"}
                      </Badge>
                      <span className="diag-summary__note">
                        {environment.ok
                          ? "Everything Shmingus Clips needs is available."
                          : `${pluralize(failingRequired, "required check")} ${
                              failingRequired === 1 ? "needs" : "need"
                            } attention.`}
                      </span>
                    </div>
                    <ul className="check-list check-list--full">
                      {environment.checks.map((check) => (
                        <li key={check.id} className="check-list__item">
                          <span
                            className={
                              check.ok
                                ? "check-list__status is-good"
                                : "check-list__status is-bad"
                            }
                            aria-hidden="true"
                          >
                            {check.ok ? (
                              <Check size={15} />
                            ) : (
                              <AlertTriangle size={15} />
                            )}
                          </span>
                          <div className="check-list__body">
                            <p className="check-list__label">
                              {check.label}
                              <span
                                className={
                                  check.required
                                    ? "check-list__tag"
                                    : "check-list__tag is-optional"
                                }
                              >
                                {check.required ? "Required" : "Optional"}
                              </span>
                            </p>
                            <p className="check-list__detail">{check.detail}</p>
                            {!check.ok && FIX_HINTS[check.id] && (
                              <p className="check-list__fix">
                                {FIX_HINTS[check.id]}
                              </p>
                            )}
                          </div>
                        </li>
                      ))}
                    </ul>
                    <details className="diag-details">
                      <summary className="diag-details__summary">Details</summary>
                      <div className="diag-details__body">
                        <ul className="diag-details__list">
                          {environment.checks.map((check) => (
                            <li key={check.id}>
                              <span className="diag-details__label">
                                {check.label}
                              </span>
                              <code className="diag-details__id">
                                {check.id}
                              </code>
                              <span className="diag-details__detail">
                                {check.detail}
                              </span>
                            </li>
                          ))}
                        </ul>
                        {environment.encoders.length > 0 && (
                          <div className="diag-encoders">
                            <span className="micro-label">
                              Available encoders
                            </span>
                            {environment.encoders.map((encoder) => (
                              <Chip key={encoder} mono>
                                {encoder}
                              </Chip>
                            ))}
                          </div>
                        )}
                        <p className="diag-details__note">
                          Session {environment.sessionType} · Desktop{" "}
                          {environment.desktop}
                        </p>
                      </div>
                    </details>
                  </>
                ) : (
                  !environmentError && (
                    <p className="muted">Checking the system…</p>
                  )
                )}
              </Panel>

              <Panel
                title="Self-test"
                description="Records and saves a short test clip without touching your screen, to prove the pieces work together."
                actions={
                  <Button
                    size="sm"
                    variant="secondary"
                    icon={selftestBusy ? <Spinner /> : <Activity size={15} />}
                    disabled={selftestBusy}
                    onClick={() => void runSelfTest()}
                  >
                    Run self-test
                  </Button>
                }
              >
                {selftestError && (
                  <Alert tone="warn" title="Self-test could not run">
                    {selftestError}
                  </Alert>
                )}
                {selftest ? (
                  <>
                    <Badge tone={selftest.ok ? "accent" : "warn"} dot>
                      {selftest.ok ? "Passed" : "Failed"}
                    </Badge>
                    {selftest.error && (
                      <Alert tone="danger">{selftest.error}</Alert>
                    )}
                    <ul className="check-list check-list--full">
                      {selftest.steps.map((step) => (
                        <li key={step.name} className="check-list__item">
                          <span
                            className={
                              step.ok
                                ? "check-list__status is-good"
                                : "check-list__status is-bad"
                            }
                            aria-hidden="true"
                          >
                            {step.ok ? (
                              <CheckCircle2 size={15} />
                            ) : (
                              <AlertTriangle size={15} />
                            )}
                          </span>
                          <div className="check-list__body">
                            <p className="check-list__label">{step.name}</p>
                            <p className="check-list__detail">{step.detail}</p>
                          </div>
                        </li>
                      ))}
                    </ul>
                    {selftest.artifacts.length > 0 && (
                      <div className="diag-artifacts">
                        <p className="micro-label">Artifacts</p>
                        {selftest.artifacts.map((artifact) => (
                          <code key={artifact} className="diag-artifacts__path">
                            {artifact}
                          </code>
                        ))}
                      </div>
                    )}
                  </>
                ) : (
                  !selftestError && (
                    <p className="muted">
                      Run the test to check recording and saving work, without
                      choosing a screen first.
                    </p>
                  )
                )}
              </Panel>
            </>
          )}

          <div className="savebar">
            <div className="savebar__status" aria-live="polite">
              {saveError ? (
                <span className="savebar__error">{saveError}</span>
              ) : savingSettings ? (
                "Saving…"
              ) : dirty ? (
                "Unsaved changes"
              ) : (
                "All changes saved"
              )}
            </div>
            <div className="savebar__actions">
              <Button
                variant="ghost"
                onClick={discard}
                disabled={!dirty || savingSettings}
              >
                Discard
              </Button>
              <Button
                variant="primary"
                icon={savingSettings ? <Spinner /> : <Check size={16} />}
                disabled={!dirty || savingSettings}
                onClick={() => void handleSave()}
              >
                {savingSettings ? "Saving…" : "Save changes"}
              </Button>
            </div>
          </div>
        </div>
      </div>

      {confirmShareId && (
        <ConfirmDialog
          title="Remove this share?"
          message="The public link stops working immediately. The clip stays on this computer."
          busy={shareBusyId === confirmShareId}
          onCancel={() => setConfirmShareId(null)}
          onConfirm={() => void removeShare(confirmShareId)}
        />
      )}

      {confirmServerShare && (
        <ConfirmDialog
          title="Remove this share?"
          message="The public link stops working immediately. This affects the account, not only this computer."
          busy={shareBusyId === confirmServerShare.id}
          onCancel={() => setConfirmServerShare(null)}
          onConfirm={() => void removeServerShare(confirmServerShare.id)}
        />
      )}
    </div>
  );
}
