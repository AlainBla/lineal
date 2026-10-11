import type { Session } from "@/app/auth";

/** Navigation der Workspace-Settings (linke Spalte): Workspace-Seiten, dann Team-Settings. */
export function workspaceSettingsNav(
  ws: string,
  session: Session | null | undefined,
  active: "workspace" | "members",
) {
  return [
    { label: "Workspace", to: `/${ws}/settings/workspace`, active: active === "workspace" },
    { label: "Members", to: `/${ws}/settings/members`, active: active === "members" },
    ...(session?.teams ?? []).map((t) => ({
      label: `Team · ${t.name}`,
      to: `/${ws}/settings/teams/${t.key}/general`,
      active: false,
    })),
  ];
}
