import type { Session } from "@/app/auth";

export type AccountSection = "profile" | "security" | "notifications" | "appearance";

/** Persönliche Seiten (Gruppe „Account“); `active` markiert die geöffnete Seite. */
export function accountNavItems(ws: string, active?: AccountSection) {
  const items: Array<{ label: string; id: AccountSection }> = [
    { label: "Profile", id: "profile" },
    { label: "Password & sessions", id: "security" },
    { label: "Notifications", id: "notifications" },
    { label: "Appearance", id: "appearance" },
  ];
  return items.map((i, n) => ({
    label: i.label,
    to: `/${ws}/settings/account/${i.id}`,
    active: i.id === active,
    // Die Überschrift steht einmal vor dem ersten Eintrag der Gruppe.
    ...(n === 0 ? { group: "Account" } : {}),
  }));
}

type WorkspaceSection =
  "workspace" | "members" | "teams" | "labels" | "templates" | "api" | "security" | "audit";

/**
 * Navigation der Settings (linke Spalte): Gruppe „Account“ (persönlich), dann die Workspace-Seiten,
 * dann Team-Settings. `active` ist eine Workspace-Seite oder `{ account: <Seite> }`.
 */
export function workspaceSettingsNav(
  ws: string,
  session: Session | null | undefined,
  active: WorkspaceSection | { account: AccountSection },
) {
  const role = session?.user.role;
  const admin = role === "owner" || role === "admin";
  const section = typeof active === "string" ? active : null;
  const accountActive = typeof active === "string" ? undefined : active.account;
  return [
    ...accountNavItems(ws, accountActive),
    {
      label: "Workspace",
      to: `/${ws}/settings/workspace`,
      active: section === "workspace",
      group: "Administration",
    },
    { label: "Members", to: `/${ws}/settings/members`, active: section === "members" },
    { label: "Teams", to: `/${ws}/settings/teams`, active: section === "teams" },
    { label: "Labels", to: `/${ws}/settings/labels`, active: section === "labels" },
    { label: "Templates", to: `/${ws}/settings/templates`, active: section === "templates" },
    ...(admin
      ? [
          { label: "Security", to: `/${ws}/settings/security`, active: section === "security" },
          { label: "Audit log", to: `/${ws}/settings/audit`, active: section === "audit" },
        ]
      : []),
    { label: "API", to: `/${ws}/settings/api`, active: section === "api" },
    ...(session?.teams ?? []).map((t) => ({
      label: `Team · ${t.name}`,
      to: `/${ws}/settings/teams/${t.key}/general`,
      active: false,
    })),
  ];
}
