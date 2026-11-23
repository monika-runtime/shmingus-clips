export type AppTheme = "clipforge" | "violet" | "cyan" | "amber";

export interface ThemeOption {
  id: AppTheme;
  label: string;
  description: string;
  /** Background, surface and accent, for the picker swatch. */
  swatch: [string, string, string];
}

export const THEMES: ThemeOption[] = [
  {
    id: "clipforge",
    label: "Lime",
    description: "Lime on charcoal",
    swatch: ["#0b0d10", "#151a20", "#c6ff4a"],
  },
  {
    id: "violet",
    label: "Violet",
    description: "Soft orchid on ink",
    swatch: ["#0b0910", "#17121f", "#c9a6ff"],
  },
  {
    id: "cyan",
    label: "Cyan lab",
    description: "Electric cyan on slate",
    swatch: ["#060b0e", "#0f181e", "#5ee7ff"],
  },
  {
    id: "amber",
    label: "Amber tape",
    description: "Warm amber on umber",
    swatch: ["#0c0a08", "#1a1510", "#ffb84d"],
  },
];

const STORAGE_KEY = "clipforge.theme";

/** The theme saved on this device, or the default. */
export function storedTheme(): AppTheme {
  try {
    const value = localStorage.getItem(STORAGE_KEY);
    if (value && THEMES.some((theme) => theme.id === value)) {
      return value as AppTheme;
    }
  } catch {
    // Storage can be unavailable; the default theme still works.
  }
  return "clipforge";
}

/** Apply a theme immediately and remember it for the next launch. */
export function applyTheme(theme: AppTheme): void {
  const root = document.documentElement;
  if (theme === "clipforge") {
    delete root.dataset.appTheme;
  } else {
    root.dataset.appTheme = theme;
  }
  try {
    localStorage.setItem(STORAGE_KEY, theme);
  } catch {
    // Remembering the choice is a convenience, not a requirement.
  }
}
