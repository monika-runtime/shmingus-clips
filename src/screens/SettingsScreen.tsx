import {
  Activity,
  AlertTriangle,
  Check,
  CheckCircle2,
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
  Volume2,
  type LucideIcon,
} from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";

import { Alert } from "../components/ui/Alert";
import { Badge, Chip } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { Field } from "../components/ui/Field";
import { Hotkey } from "../components/ui/Hotkey";
import { Input } from "../components/ui/Input";
import { PageHeader } from "../components/ui/PageHeader";
import { Segmented } from "../components/ui/Segmented";
import { Select } from "../components/ui/Select";
import { Slider } from "../components/ui/Slider";
import { Spinner } from "../components/ui/Spinner";
import { Switch } from "../components/ui/Switch";
import { backend, chooseDirectory } from "../lib/bridge";
import { DEFAULT_HOTKEYS } from "../lib/defaults";
import { errorMessage } from "../lib/errors";
import { formatBytes, pluralize, signature } from "../lib/format";
import { previewAuthSample } from "../lib/preview";
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
  { id: "diagnostics", label: "Diagnostics", icon: Activity },
];

const BUFFER_OPTIONS = [15, 30, 60, 120].map((value) => ({
  value,
  label: `${value}s`,
}));

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
  label,
  title,
  description,
  actions,
  children,
}: {
  label: string;
  title: string;
  description?: string;
  actions?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="card settings-panel">
      <header className="settings-panel__head">
        <div className="settings-panel__heading">
          <p className="micro-label">{label}</p>
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
    environment,
    environmentError,
    environmentBusy,
    refreshEnvironment,
    pushToast,
  } = useApp();

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
                label="Capture"
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
                      value={draft.bufferSeconds}
                      options={BUFFER_OPTIONS}
                      onChange={(value) => patch({ bufferSeconds: value })}
                    />
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
                label="Detection"
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
              label="Audio"
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
            </Panel>
          )}

          {settingsSection === "hotkeys" && (
            <Panel
              label="Hotkeys"
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
              label="Storage"
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
                  <span className="usage__value">Clipforge/Exports</span>
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
                label="Sharing"
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
                label="Sharing"
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
                label="Sharing"
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
                label="Appearance"
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
                label="Application"
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
              </Panel>
            </>
          )}

          {settingsSection === "diagnostics" && (
            <>
              <Panel
                label="Diagnostics"
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
                label="Diagnostics"
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
    </div>
  );
}
