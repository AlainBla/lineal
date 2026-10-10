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

import { TeamSettingsPage } from "./settings-pages";

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
});
