import "@/app/test-polyfills";

import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { SESSION_QUERY_KEY } from "@/app/auth";
import {
  mockGraphql,
  newClient,
  renderWithApp,
  SESSION,
  STATES,
  TEAM_META,
} from "@/features/issues/test-fixtures";

import { TeamSettingsPage, WorkspaceSettingsPage } from "./settings-pages";

afterEach(() => vi.unstubAllGlobals());

const handlers = () => ({
  SettingsTeam: () => ({ team: TEAM_META.team }),
  SettingsTeamUpdate: () => ({ teamUpdate: { success: true } }),
  SettingsStates: () => ({ workflowStates: STATES }),
  SettingsStateCreate: () => ({ workflowStateCreate: { success: true } }),
  SettingsStateUpdate: () => ({ workflowStateUpdate: { success: true } }),
});

describe("TeamSettingsPage", () => {
  it("saves changed general fields only", async () => {
    const gql = mockGraphql(handlers());
    await renderWithApp(
      <TeamSettingsPage ws="acme" teamKey="ENG" section="general" />,
      newClient(),
      "/acme/settings/teams/ENG/general",
    );
    const user = userEvent.setup();
    const name = await screen.findByLabelText("Name");
    await user.clear(name);
    await user.type(name, "Platform");
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(gql.of("SettingsTeamUpdate")).toHaveLength(1));
    expect(gql.of("SettingsTeamUpdate")[0]!.variables).toEqual({
      id: "team-eng",
      input: { name: "Platform" },
    });
  });

  it("sends an explicit null when icon and color are emptied", async () => {
    const gql = mockGraphql({
      ...handlers(),
      SettingsTeam: () => ({ team: { ...TEAM_META.team, icon: "rocket", color: "#112233" } }),
    });
    await renderWithApp(
      <TeamSettingsPage ws="acme" teamKey="ENG" section="general" />,
      newClient(),
      "/acme/settings/teams/ENG/general",
    );
    const user = userEvent.setup();
    await user.clear(await screen.findByLabelText("Icon"));
    await user.clear(screen.getByLabelText("Color"));
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(gql.of("SettingsTeamUpdate")).toHaveLength(1));
    expect(gql.of("SettingsTeamUpdate")[0]!.variables).toEqual({
      id: "team-eng",
      input: { icon: null, color: null },
    });
  });

  it("workflow editor groups by category and creates a status", async () => {
    const gql = mockGraphql(handlers());
    await renderWithApp(
      <TeamSettingsPage ws="acme" teamKey="ENG" section="statuses" />,
      newClient(),
      "/acme/settings/teams/ENG/statuses",
    );
    const user = userEvent.setup();
    const started = await screen.findByRole("group", { name: "Started" });
    expect(within(started).getByText("In Progress")).toBeInTheDocument();
    await user.type(screen.getByLabelText("New status name"), "In Review");
    await user.selectOptions(screen.getByLabelText("Category"), "started");
    await user.click(screen.getByRole("button", { name: /Create status/ }));
    await waitFor(() => expect(gql.of("SettingsStateCreate")).toHaveLength(1));
    expect(gql.of("SettingsStateCreate")[0]!.variables).toEqual({
      input: { teamId: "team-eng", name: "In Review", type: "started", color: "#5e6ad2" },
    });
  });

  it("moving a status sends a position between its neighbours", async () => {
    const gql = mockGraphql(handlers());
    await renderWithApp(
      <TeamSettingsPage ws="acme" teamKey="ENG" section="statuses" />,
      newClient(),
      "/acme/settings/teams/ENG/statuses",
    );
    await screen.findByRole("group", { name: "Completed" });
    // Backlog (pos 0) und Todo (pos 1) sind je einzeln in ihrer Kategorie: nichts zu verschieben.
    expect(screen.getByRole("button", { name: "Move Todo up" })).toBeDisabled();
    expect(gql.of("SettingsStateUpdate")).toHaveLength(0);
  });

  it("LIN-73: picking a palette color for a status persists only the color", async () => {
    const gql = mockGraphql(handlers());
    await renderWithApp(
      <TeamSettingsPage ws="acme" teamKey="ENG" section="statuses" />,
      newClient(),
      "/acme/settings/teams/ENG/statuses",
    );
    const user = userEvent.setup();
    // Farbwähler der Zeile „In Progress" öffnen und eine Palettenfarbe (Red, #eb5757) wählen.
    await user.click(await screen.findByRole("button", { name: "Color for In Progress" }));
    await user.click(await screen.findByRole("button", { name: "Red" }));
    await waitFor(() => expect(gql.of("SettingsStateUpdate")).toHaveLength(1));
    expect(gql.of("SettingsStateUpdate")[0]!.variables).toEqual({
      id: "s-progress",
      input: { color: "#eb5757" },
    });
  });

  it("LIN-73: typing a custom hex and applying persists the color", async () => {
    const gql = mockGraphql(handlers());
    await renderWithApp(
      <TeamSettingsPage ws="acme" teamKey="ENG" section="statuses" />,
      newClient(),
      "/acme/settings/teams/ENG/statuses",
    );
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Color for Done" }));
    const hex = await screen.findByLabelText("Hex color");
    await user.clear(hex);
    await user.type(hex, "#123abc");
    await user.click(screen.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(gql.of("SettingsStateUpdate")).toHaveLength(1));
    expect(gql.of("SettingsStateUpdate")[0]!.variables).toEqual({
      id: "s-done",
      input: { color: "#123abc" },
    });
  });

  it("LIN-73: reordering a status persists the position and reflects the new order", async () => {
    // Zwei „started"-Status, damit innerhalb der Kategorie sortiert werden kann; Handler ist
    // zustandsbehaftet, damit der Refetch nach der Mutation die neue Reihenfolge zurückgibt.
    const started = STATES.find((s) => s.type === "started")!;
    let rows = [
      { ...started, id: "s-progress", name: "In Progress", position: 2 },
      { ...started, id: "s-review", name: "In Review", position: 3 },
    ];
    const gql = mockGraphql({
      ...handlers(),
      SettingsStates: () => ({ workflowStates: [...rows] }),
      SettingsStateUpdate: (v) => {
        const input = v.input as { position?: number };
        if (typeof input.position === "number") {
          const pos = input.position;
          rows = rows.map((s) => (s.id === v.id ? { ...s, position: pos } : s));
        }
        return { workflowStateUpdate: { success: true } };
      },
    });
    await renderWithApp(
      <TeamSettingsPage ws="acme" teamKey="ENG" section="statuses" />,
      newClient(),
      "/acme/settings/teams/ENG/statuses",
    );
    const user = userEvent.setup();
    const group = await screen.findByRole("group", { name: "Started" });
    // Ausgangsreihenfolge: In Progress, In Review.
    expect(
      within(group)
        .getAllByRole("listitem")
        .map((li) => li.textContent),
    ).toEqual([expect.stringContaining("In Progress"), expect.stringContaining("In Review")]);
    await user.click(within(group).getByRole("button", { name: "Move In Progress down" }));
    await waitFor(() => expect(gql.of("SettingsStateUpdate")).toHaveLength(1));
    // In Progress (pos 2) hinter In Review (pos 3): neue Position > 3.
    expect(gql.of("SettingsStateUpdate")[0]!.variables).toEqual({
      id: "s-progress",
      input: { position: 4 },
    });
    // Nach dem Refetch spiegelt die Liste die neue Reihenfolge.
    await waitFor(() => {
      const started2 = screen.getByRole("group", { name: "Started" });
      expect(
        within(started2)
          .getAllByRole("listitem")
          .map((li) => li.textContent),
      ).toEqual([expect.stringContaining("In Review"), expect.stringContaining("In Progress")]);
    });
  });

  it("B5.02: cycles form shows the team schedule and only sends changed values", async () => {
    const gql = mockGraphql({
      ...handlers(),
      SettingsCyclesUpdate: () => ({ teamCyclesUpdate: { success: true } }),
      SettingsTeam: () => ({
        team: {
          ...TEAM_META.team,
          cyclesEnabled: true,
          cycleDuration: 3,
          cycleCooldownTime: 1,
          cycleStartDay: 4,
          upcomingCycleCount: 4,
        },
      }),
    });
    await renderWithApp(
      <TeamSettingsPage ws="acme" teamKey="ENG" section="cycles" />,
      newClient(),
      "/acme/settings/teams/ENG/cycles",
    );
    const user = userEvent.setup();
    expect(await screen.findByLabelText("Cycle duration")).toHaveValue("3");
    expect(screen.getByLabelText("Cooldown")).toHaveValue("1");
    expect(screen.getByLabelText("Start day")).toHaveValue("4");
    expect(screen.getByLabelText("Upcoming cycles")).toHaveValue("4");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(gql.of("SettingsCyclesUpdate")).toHaveLength(0);
    await user.selectOptions(screen.getByLabelText("Cooldown"), "8");
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(gql.of("SettingsCyclesUpdate")).toHaveLength(1));
    expect(gql.of("SettingsCyclesUpdate")[0]!.variables).toEqual({
      teamId: "team-eng",
      input: { cycleCooldownTime: 8 },
    });
  });

  it("B5.06: fiscal year start is shown by month and saved 0-11", async () => {
    const gql = mockGraphql({
      SettingsWorkspace: () => ({
        workspace: {
          id: "o1",
          name: "Acme",
          urlKey: "acme",
          fiscalYearStartMonth: 0,
          initiativesEnabled: true,
          logoUrl: null,
        },
      }),
      SettingsWorkspaceUpdate: () => ({ workspaceUpdate: { success: true } }),
    });
    const admin = newClient();
    admin.setQueryData(SESSION_QUERY_KEY, { ...SESSION, user: { ...SESSION.user, role: "admin" } });
    await renderWithApp(<WorkspaceSettingsPage ws="acme" />, admin, "/acme/settings/workspace");
    const user = userEvent.setup();
    const select = await screen.findByLabelText("Fiscal year start");
    expect(select).toHaveValue("0");
    await user.selectOptions(select, "December");
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(gql.of("SettingsWorkspaceUpdate")).toHaveLength(1));
    expect(gql.of("SettingsWorkspaceUpdate")[0]!.variables).toEqual({
      input: { fiscalYearStartMonth: 11 },
    });
  });

  it("B5.35: removing a member asks first; the picker is sorted and searchable", async () => {
    const gql = mockGraphql({
      ...handlers(),
      SettingsTeamMemberInfos: () => ({
        teamMembers: [{ id: "u1", name: "zed", displayName: "Zed", owner: false, role: "user" }],
      }),
      SettingsWorkspaceMembers: () => ({
        users: [
          { id: "u1", name: "zed", displayName: "Zed" },
          { id: "u3", name: "carl", displayName: "Carl" },
          { id: "u2", name: "anna", displayName: "Anna" },
        ],
      }),
      SettingsMemberRemove: () => ({ teamMembershipDelete: { success: true } }),
    });
    const confirm = vi.spyOn(window, "confirm").mockReturnValueOnce(false).mockReturnValue(true);
    await renderWithApp(
      <TeamSettingsPage ws="acme" teamKey="ENG" section="members" />,
      newClient(),
      "/acme/settings/teams/ENG/members",
    );
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Remove Zed" }));
    expect(gql.of("SettingsMemberRemove")).toHaveLength(0);
    await user.click(screen.getByRole("button", { name: "Remove Zed" }));
    await waitFor(() => expect(gql.of("SettingsMemberRemove")).toHaveLength(1));
    expect(confirm).toHaveBeenCalledTimes(2);
    await user.click(screen.getByRole("button", { name: /Add a member/ }));
    const select = await screen.findByLabelText("Add member");
    await waitFor(() =>
      expect(
        within(select)
          .getAllByRole("option")
          .map((o) => o.textContent),
      ).toEqual(["Choose a person…", "Anna", "Carl"]),
    );
    await user.type(screen.getByLabelText("Search people"), "car");
    expect(
      within(select)
        .getAllByRole("option")
        .map((o) => o.textContent),
    ).toEqual(["Choose a person…", "Carl"]);
    confirm.mockRestore();
  });

  it("F.14: owners are shown and can be appointed or revoked; guests have no owner button", async () => {
    const gql = mockGraphql({
      ...handlers(),
      SettingsTeam: () => ({
        team: { ...TEAM_META.team, viewer: { isMember: true, isOwner: true, canJoin: false } },
      }),
      SettingsTeamMemberInfos: () => ({
        teamMembers: [
          { id: "u1", name: "zed", displayName: "Zed", owner: false, role: "user" },
          { id: "u2", name: "ann", displayName: "Ann", owner: true, role: "user" },
          { id: "u3", name: "gus", displayName: "Gus", owner: false, role: "guest" },
        ],
      }),
      SettingsTeamOwnerSet: () => ({ teamMembershipUpdate: { success: true } }),
    });
    await renderWithApp(
      <TeamSettingsPage ws="acme" teamKey="ENG" section="members" />,
      newClient(),
      "/acme/settings/teams/ENG/members",
    );
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Make owner Zed" }));
    await waitFor(() => expect(gql.of("SettingsTeamOwnerSet")).toHaveLength(1));
    expect(gql.of("SettingsTeamOwnerSet")[0]!.variables).toMatchObject({
      userId: "u1",
      owner: true,
    });
    expect(screen.getByRole("button", { name: "Revoke owner from Ann" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /owner.*Gus/ })).toBeNull();
  });

  it("F.19: join button appears when canJoin and leave button for members", async () => {
    const gql = mockGraphql({
      ...handlers(),
      SettingsTeam: () => ({
        team: { ...TEAM_META.team, viewer: { isMember: false, isOwner: false, canJoin: true } },
      }),
      SettingsTeamMemberInfos: () => ({ teamMembers: [] }),
      SettingsTeamJoin: () => ({ teamMembershipCreate: { success: true } }),
    });
    await renderWithApp(
      <TeamSettingsPage ws="acme" teamKey="ENG" section="members" />,
      newClient(),
      "/acme/settings/teams/ENG/members",
    );
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Join team" }));
    await waitFor(() => expect(gql.of("SettingsTeamJoin")).toHaveLength(1));
    expect(gql.of("SettingsTeamJoin")[0]!.variables).toMatchObject({ userId: SESSION.user.id });
    expect(screen.queryByRole("button", { name: "Leave team" })).toBeNull();
  });

  it("B5.36: non-admins see the workspace form read-only", async () => {
    mockGraphql({
      SettingsWorkspace: () => ({
        workspace: {
          id: "o1",
          name: "Acme",
          urlKey: "acme",
          fiscalYearStartMonth: 0,
          initiativesEnabled: true,
          logoUrl: null,
        },
      }),
    });
    const member = newClient();
    member.setQueryData(SESSION_QUERY_KEY, {
      ...SESSION,
      user: { ...SESSION.user, role: "user" },
    });
    await renderWithApp(<WorkspaceSettingsPage ws="acme" />, member, "/acme/settings/workspace");
    expect(await screen.findByLabelText("Workspace name")).toBeDisabled();
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  });
});
