import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { mockGraphql, newClient, renderWithApp, TEAM_META } from "@/features/issues/test-fixtures";

import { TeamSettingsPage } from "./settings-pages";
import type { TeamRow } from "./teams-queries";

afterEach(() => vi.unstubAllGlobals());

const row = (over: Partial<TeamRow> & Pick<TeamRow, "id" | "key" | "name">): TeamRow => ({
  parentId: null,
  icon: null,
  color: null,
  visibility: "public",
  allMembersCanJoin: true,
  retiredAt: null,
  createdAt: "2026-01-01T00:00:00Z",
  memberCount: 1,
  ...over,
});

const base = (teams: TeamRow[], team = TEAM_META.team) => ({
  SettingsTeam: () => ({ team }),
  SettingsTeams: () => ({ teams }),
  SettingsTeamMembers: () => ({ users: [] }),
  SettingsTeamUpdate: () => ({ teamUpdate: { success: true } }),
  SettingsTeamRetire: () => ({ teamRetire: { success: true } }),
  SettingsTeamUnretire: () => ({ teamUnretire: { success: true } }),
  SettingsTeamDelete: () => ({ teamDelete: { success: true } }),
});

const ENG = row({ id: "team-eng", key: "ENG", name: "Engineering" });
const WEB = row({ id: "team-web", key: "WEB", name: "Web", parentId: "team-eng" });

async function render(section: "general" | "members" = "general") {
  await renderWithApp(
    <TeamSettingsPage ws="acme" teamKey="ENG" section={section} />,
    newClient(),
    `/acme/settings/teams/ENG/${section}`,
  );
  return userEvent.setup();
}

describe("Team danger zone", () => {
  it("retires a team without sub-teams directly", async () => {
    const gql = mockGraphql(base([ENG]));
    const user = await render();
    await user.click(await screen.findByRole("button", { name: "Retire team…" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Retire team" }));
    await waitFor(() => expect(gql.of("SettingsTeamRetire")).toHaveLength(1));
    expect(gql.of("SettingsTeamRetire")[0]!.variables).toEqual({ id: "team-eng" });
  });

  it("asks what happens to active sub-teams before retiring", async () => {
    const gql = mockGraphql(base([ENG, WEB]));
    const user = await render();
    await user.click(await screen.findByRole("button", { name: "Retire team…" }));
    const dialog = await screen.findByRole("dialog");
    await within(dialog).findByText(/1 active sub-team/);
    const confirm = within(dialog).getByRole("button", { name: "Retire team" });
    expect(confirm).toBeDisabled();
    await user.selectOptions(within(dialog).getByLabelText("Sub-teams"), "unnest");
    await user.click(confirm);
    await waitFor(() => expect(gql.of("SettingsTeamRetire")).toHaveLength(1));
    expect(gql.of("SettingsTeamRetire")[0]!.variables).toEqual({
      id: "team-eng",
      handleSubTeamsOnRetirement: "unnest",
    });
  });

  it("shows the server error when retiring is refused", async () => {
    mockGraphql({ ...base([ENG]), SettingsTeamRetire: () => new Error("Only owners may retire") });
    const user = await render();
    await user.click(await screen.findByRole("button", { name: "Retire team…" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Retire team" }));
    expect(await within(dialog).findByText("Only owners may retire")).toBeInTheDocument();
  });

  it("offers to reactivate a retired team", async () => {
    const gql = mockGraphql(base([ENG], { ...TEAM_META.team, retiredAt: "2026-03-01T00:00:00Z" }));
    const user = await render();
    expect(screen.queryByRole("button", { name: "Retire team…" })).not.toBeInTheDocument();
    await user.click(await screen.findByRole("button", { name: "Reactivate team" }));
    await waitFor(() => expect(gql.of("SettingsTeamUnretire")).toHaveLength(1));
    expect(gql.of("SettingsTeamUnretire")[0]!.variables).toEqual({ id: "team-eng" });
  });

  it("deletes only after the team key was typed", async () => {
    const gql = mockGraphql(base([ENG]));
    const user = await render();
    await user.click(await screen.findByRole("button", { name: "Delete team…" }));
    const dialog = await screen.findByRole("dialog");
    const confirm = within(dialog).getByRole("button", { name: "Delete team" });
    expect(confirm).toBeDisabled();
    await user.type(within(dialog).getByLabelText("Confirm team identifier"), "eng");
    expect(confirm).toBeDisabled();
    await user.clear(within(dialog).getByLabelText("Confirm team identifier"));
    await user.type(within(dialog).getByLabelText("Confirm team identifier"), "ENG");
    expect(confirm).toBeEnabled();
    await user.click(confirm);
    await waitFor(() => expect(gql.of("SettingsTeamDelete")).toHaveLength(1));
    expect(gql.of("SettingsTeamDelete")[0]!.variables).toEqual({ id: "team-eng" });
  });

  it("changing the identifier needs a confirmation", async () => {
    const gql = mockGraphql(base([ENG]));
    const user = await render();
    const key = await screen.findByLabelText("Identifier");
    await user.clear(key);
    await user.type(key, "core");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(gql.of("SettingsTeamUpdate")).toHaveLength(0);
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/old\s+identifiers keep resolving/)).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Change identifier" }));
    await waitFor(() => expect(gql.of("SettingsTeamUpdate")).toHaveLength(1));
    expect(gql.of("SettingsTeamUpdate")[0]!.variables).toEqual({
      id: "team-eng",
      input: { key: "CORE" },
    });
  });

  it("saves visibility and join rule from the access section", async () => {
    const gql = mockGraphql(base([ENG]));
    const user = await render("members");
    await user.selectOptions(await screen.findByLabelText("Team visibility"), "private");
    await user.click(screen.getByLabelText("All members can join"));
    const form = screen.getByLabelText("Team visibility").closest("form")!;
    await user.click(within(form).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(gql.of("SettingsTeamUpdate")).toHaveLength(1));
    expect(gql.of("SettingsTeamUpdate")[0]!.variables).toEqual({
      id: "team-eng",
      input: { visibility: "private", allMembersCanJoin: false },
    });
  });
});
