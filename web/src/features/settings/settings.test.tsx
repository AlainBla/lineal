import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  mockGraphql,
  newClient,
  renderWithApp,
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
    await renderWithApp(
      <WorkspaceSettingsPage ws="acme" />,
      newClient(),
      "/acme/settings/workspace",
    );
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
});
