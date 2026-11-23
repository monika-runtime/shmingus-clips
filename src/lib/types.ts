// Mirror of `src-tauri/src/model.rs`. Keep both in sync.

export type EncoderKind = "auto" | "nvenc" | "x264";

export interface Hotkeys {
  saveClip: string;
  toggleBuffer: string;
  toggleMic: string;
}

export interface AudioSettings {
  desktopEnabled: boolean;
  desktopVolume: number;
  micEnabled: boolean;
  micDevice: string | null;
  micVolume: number;
  /** Experimental: lift a quiet mic toward the desktop level in the mix. */
  micAutoGain: boolean;
}

export type EditorAspect = "source" | "vertical" | "square";

export interface EditorSegment {
  clipId: string;
  startMs: number;
  endMs: number;
  speed: number;
  desktopGain: number;
  micGain: number;
  muteDesktop: boolean;
  muteMic: boolean;
  fadeInMs: number;
  fadeOutMs: number;
  rotation: 0 | 90 | 180 | 270;
}

export interface EditorOverlay {
  text: string;
  startMs: number;
  endMs: number;
  x: number;
  y: number;
  size: number;
  color: string;
  boxed: boolean;
}

export interface EditPlan {
  title: string;
  aspect: EditorAspect;
  segments: EditorSegment[];
  overlays: EditorOverlay[];
}

export interface SavedMonitor {
  id: number;
  name: string;
  restoreToken: string | null;
}

export interface Settings {
  saveDir: string;
  bufferSeconds: number;
  fps: number;
  bitrateMbps: number;
  encoder: EncoderKind;
  audio: AudioSettings;
  hotkeys: Hotkeys;
  captureMonitor: SavedMonitor | null;
  autoDetectGames: boolean;
  /** Start the buffer when a game is detected; stop it when the game exits. */
  autoStartOnGame: boolean;
  /** Save the buffered moments when a detected game exits (quit or crash). */
  saveOnGameExit: boolean;
  startBufferOnLaunch: boolean;
  minimizeToTray: boolean;
  launchAtLogin: boolean;
  notifyOnClip: boolean;
  /** Play a short chime when a clip is saved. */
  clipSound: boolean;
  /** Base URL of the self-hosted share service, empty when sharing is off. */
  shareBaseUrl: string;
  /** Optional upload token for the share service. */
  shareToken: string | null;
  /** Compatibility: draw the interface with software rendering. */
  softwareRendering: boolean;
  maxWidth: number;
}

export type RecorderState = "off" | "starting" | "buffering" | "saving" | "error";

export interface RecorderStatus {
  state: RecorderState;
  encoder: string | null;
  hardwareEncoding: boolean;
  captureLabel: string | null;
  bufferedSeconds: number;
  bufferTargetSeconds: number;
  startedAt: string | null;
  micActive: boolean;
  desktopAudioActive: boolean;
  error: string | null;
}

export interface MicLevel {
  rmsDb: number;
  peakDb: number;
}

export interface AudioDevice {
  id: string;
  label: string;
  isMonitor: boolean;
  isDefault: boolean;
}

export type GameSource = "steam" | "lutris" | "heroic" | "process" | "manual";

export interface GameInfo {
  id: string;
  name: string;
  source: GameSource;
  pid: number | null;
}

export interface Clip {
  id: string;
  title: string;
  game: string | null;
  path: string;
  thumbnail: string | null;
  createdAt: string;
  durationMs: number;
  sizeBytes: number;
  width: number;
  height: number;
  favorite: boolean;
}

export interface CheckResult {
  id: string;
  label: string;
  ok: boolean;
  required: boolean;
  detail: string;
}

export interface EnvironmentReport {
  ok: boolean;
  checks: CheckResult[];
  encoders: string[];
  sessionType: string;
  desktop: string;
}

export interface SelfTestStep {
  name: string;
  ok: boolean;
  detail: string;
}

export interface SelfTestReport {
  ok: boolean;
  steps: SelfTestStep[];
  artifacts: string[];
  error: string | null;
}

export interface ShareInfo {
  id: string;
  url: string;
  embedUrl: string;
  videoUrl: string;
  createdAt: string;
}

export interface AuthStatus {
  signedIn: boolean;
  name: string | null;
  email: string | null;
  /** Authentik sub (username). */
  sub: string | null;
  /** RFC 3339 access-token expiry. */
  expiresAt: string | null;
}

export interface ShareServerStatus {
  ok: boolean;
  detail: string;
}

export interface Toast {
  level: "info" | "success" | "error";
  message: string;
}
