import type { Session } from "@/app/auth";

/** Navigation der Workspace-Settings (linke Spalte): Workspace-Seiten, dann Team-Settings. */
export function workspaceSettingsNav(
  ws: string,
  session: Session | null | undefined,
  active: "workspace" | "members" | "teams" | "api" | "security" | "audit",
) {
  const role = session?.user.role;
  const admin = role === "owner" || role === "admin";
  return [
    { label: "Workspace", to: `/${ws}/settings/workspace`, active: active === "workspace" },
    { label: "Members", to: `/${ws}/settings/members`, active: active === "members" },
    { label: "Teams", to: `/${ws}/settings/teams`, active: active === "teams" },
    ...(admin
      ? [
          { label: "Security", to: `/${ws}/settings/security`, active: active === "security" },
          { label: "Audit log", to: `/${ws}/settings/audit`, active: active === "audit" },
        ]
      : []),
    { label: "API", to: `/${ws}/settings/api`, active: active === "api" },
    ...(session?.teams ?? []).map((t) => ({
      label: `Team · ${t.name}`,
      to: `/${ws}/settings/teams/${t.key}/general`,
      active: false,
    })),
  ];
}
