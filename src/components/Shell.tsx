import {
  Library,
  Settings as SettingsIcon,
  type LucideIcon,
} from "lucide-react";
import { useEffect, useRef, type ReactNode } from "react";

import { APP_VERSION } from "../lib/defaults";
import { useApp, type Route } from "../state/store";
import { Toasts } from "./ui/Toasts";
import { SetupBanner } from "./SetupBanner";

const NAV: Array<{ route: Route; label: string; icon: LucideIcon }> = [
  { route: "library", label: "Library", icon: Library },
  { route: "settings", label: "Settings", icon: SettingsIcon },
];

export function Shell({ children }: { children: ReactNode }) {
  const { mode, route, navigate, settingsSection, toasts, dismissToast } =
    useApp();
  const mainRef = useRef<HTMLElement>(null);

  useEffect(() => {
    mainRef.current?.scrollTo({ top: 0, behavior: "auto" });
  }, [route, settingsSection]);

  return (
    <div className="app">
      <a
        className="skip-link"
        href="#main"
        onClick={(event) => {
          event.preventDefault();
          document.getElementById("main")?.focus();
        }}
      >
        Skip to content
      </a>
      <aside className="rail">
        <button
          type="button"
          className={route === "dashboard" ? "rail__brand is-active" : "rail__brand"}
          onClick={() => navigate("dashboard")}
          aria-label="Dashboard"
          aria-current={route === "dashboard" ? "page" : undefined}
          title="Dashboard"
        >
          <span className="rail__brand-mark" aria-hidden="true">
            <svg viewBox="0 0 24 24" fill="currentColor">
              <path d="M8 5.5v13l10.5-6.5L8 5.5Z" />
            </svg>
          </span>
          <span className="rail__brand-label">Dashboard</span>
        </button>

        <nav className="rail__nav" aria-label="Primary">
          {NAV.map(({ route: target, label, icon: Icon }) => {
            const active = route === target;
            return (
              <button
                key={target}
                type="button"
                className={active ? "rail__item is-active" : "rail__item"}
                aria-current={active ? "page" : undefined}
                onClick={() => navigate(target)}
              >
                <Icon size={19} aria-hidden="true" />
                <span className="rail__label">{label}</span>
              </button>
            );
          })}
        </nav>

        <div className="rail__bottom">
          <span className="rail__version">v{APP_VERSION}</span>
          {mode === "preview" && (
            <span className="rail__preview" title="Seed data - no Tauri runtime">
              Preview
            </span>
          )}
        </div>
      </aside>

      <main className="main" id="main" ref={mainRef} tabIndex={-1}>
        <SetupBanner />
        <div className="main__content">{children}</div>
      </main>

      <Toasts toasts={toasts} onDismiss={dismissToast} />
    </div>
  );
}
