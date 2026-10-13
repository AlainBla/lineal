/**
 * Kleiner globaler UI-Zustand (Sidebar, Overlays). Andere WPs lesen/setzen ihn über diese Schnittstelle:
 * z. B. öffnet WP13 das Create-Issue-Modal, sobald `createIssueOpen` wahr wird (Aktion `issue.create`).
 */
import { useSyncExternalStore } from "react";

export interface UiState {
  sidebarOpen: boolean;
  commandMenuOpen: boolean;
  shortcutHelpOpen: boolean;
  createIssueOpen: boolean;
  /** Dialog „New team“ (Sidebar „+“, Command-Menü, Settings → Teams). */
  createTeamOpen: boolean;
  /** Rechte Detail-Sidebar (`]`); die Inhalte liefern die Feature-WPs. */
  detailsOpen: boolean;
}

export const SIDEBAR_STORAGE_KEY = "lineal-sidebar-open";

function initialSidebar(): boolean {
  try {
    return localStorage.getItem(SIDEBAR_STORAGE_KEY) !== "false";
  } catch {
    return true;
  }
}

let state: UiState = {
  sidebarOpen: initialSidebar(),
  commandMenuOpen: false,
  shortcutHelpOpen: false,
  createIssueOpen: false,
  createTeamOpen: false,
  detailsOpen: false,
};

const listeners = new Set<() => void>();

export function getUiState(): UiState {
  return state;
}

/** `persist: false`: Änderung gilt nur für diese Sitzung (z. B. automatisches Einklappen auf schmalen Viewports). */
export function setUiState(patch: Partial<UiState>, opts: { persist?: boolean } = {}) {
  state = { ...state, ...patch };
  if ("sidebarOpen" in patch && opts.persist !== false) {
    try {
      localStorage.setItem(SIDEBAR_STORAGE_KEY, String(state.sidebarOpen));
    } catch {
      /* ignorieren */
    }
  }
  listeners.forEach((l) => l());
}

export function toggleSidebar(persist = true) {
  setUiState({ sidebarOpen: !state.sidebarOpen }, { persist });
}

export function toggleCommandMenu(open = !state.commandMenuOpen) {
  setUiState({ commandMenuOpen: open, ...(open ? { shortcutHelpOpen: false } : {}) });
}

export function toggleShortcutHelp(open = !state.shortcutHelpOpen) {
  setUiState({ shortcutHelpOpen: open, ...(open ? { commandMenuOpen: false } : {}) });
}

function subscribe(l: () => void) {
  listeners.add(l);
  return () => listeners.delete(l);
}

export function useUiState(): UiState {
  return useSyncExternalStore(subscribe, getUiState, getUiState);
}
