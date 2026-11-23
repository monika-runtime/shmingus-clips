const dateTime = new Intl.DateTimeFormat(undefined, {
  dateStyle: "medium",
  timeStyle: "short",
});

const relative = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });

const relativeUnits: Array<[Intl.RelativeTimeFormatUnit, number]> = [
  ["year", 365 * 24 * 60 * 60 * 1000],
  ["month", 30 * 24 * 60 * 60 * 1000],
  ["day", 24 * 60 * 60 * 1000],
  ["hour", 60 * 60 * 1000],
  ["minute", 60 * 1000],
];

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 MB";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const exponent = Math.min(
    units.length - 1,
    Math.floor(Math.log(bytes) / Math.log(1024)),
  );
  const value = bytes / 1024 ** exponent;
  const digits = exponent >= 3 ? 2 : value >= 100 ? 0 : 1;
  return `${value.toFixed(digits)} ${units[exponent]}`;
}

/** 42.6s -> "0:42", 1h 4m -> "1:04:12" */
export function formatDuration(ms: number): string {
  const total = Math.max(0, Math.round(ms / 1000));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = total % 60;
  if (hours > 0) {
    return `${hours}:${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
  }
  return `${minutes}:${String(seconds).padStart(2, "0")}`;
}

export function formatDateTime(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return dateTime.format(date);
}

/** True when the timestamp falls on the current local calendar day. */
export function isToday(iso: string): boolean {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return false;
  const now = new Date();
  return (
    date.getFullYear() === now.getFullYear() &&
    date.getMonth() === now.getMonth() &&
    date.getDate() === now.getDate()
  );
}

export function formatRelative(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  const diff = date.getTime() - Date.now();
  const absolute = Math.abs(diff);
  if (absolute < 60 * 1000) return "just now";
  for (const [unit, size] of relativeUnits) {
    if (absolute >= size) {
      return relative.format(Math.round(diff / size), unit);
    }
  }
  return "just now";
}

export function formatResolution(width: number, height: number): string {
  if (!width || !height) return "-";
  return `${width}×${height}`;
}

const KEY_LABELS: Record<string, string> = {
  CTRL: "Ctrl",
  CONTROL: "Ctrl",
  ALT: "Alt",
  SHIFT: "Shift",
  SUPER: "Super",
  META: "Meta",
  LOGO: "Super",
  WIN: "Super",
};

/** "CTRL+ALT+c" -> ["Ctrl", "Alt", "C"] */
export function formatAccelerator(accelerator: string): string[] {
  return accelerator
    .split("+")
    .map((part) => part.trim())
    .filter(Boolean)
    .map((part) => {
      const upper = part.toUpperCase();
      if (KEY_LABELS[upper]) return KEY_LABELS[upper];
      if (part.length === 1) return part.toUpperCase();
      return part.charAt(0).toUpperCase() + part.slice(1);
    });
}

export function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

/**
 * "1 clip" / "3 clips". `plural` defaults to `singular + "s"`; pass it for
 * irregular words ("1 required check" works, but "1 child" would not).
 */
export function pluralize(
  count: number,
  singular: string,
  plural = `${singular}s`,
): string {
  return `${count} ${count === 1 ? singular : plural}`;
}

/** Order-independent JSON signature, used to detect settings changes. */
export function signature(value: unknown): string {
  return JSON.stringify(value, (_key, nested: unknown) => {
    if (nested && typeof nested === "object" && !Array.isArray(nested)) {
      return Object.fromEntries(
        Object.entries(nested as Record<string, unknown>).sort(([a], [b]) =>
          a.localeCompare(b),
        ),
      );
    }
    return nested;
  });
}
