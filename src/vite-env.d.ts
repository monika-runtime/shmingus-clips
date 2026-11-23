/// <reference types="vite/client" />

interface Window {
  /** Injected by the Tauri runtime; absent in a plain browser. */
  __TAURI_INTERNALS__?: unknown;
}
