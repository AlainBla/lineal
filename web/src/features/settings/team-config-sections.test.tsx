import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  mockGraphql,
  newClient,
  renderWithApp,
  STATES,
  TEAM_META,
} from "@/features/issues/test-fixtures";

import { TeamSettingsPage, type TeamSection } from "./settings-pages";

afterEach(() => vi.unstubAllGlobals());

const baseHandlers = (team: Record<string, unknown> = {}, update?: () => unknown) => ({
  SettingsTeam: () => ({ team: { ...TEAM_META.team, ...team } }),
  SettingsTeamUpdate: update ?? (() => ({ teamUpdate: { success: true } })),
  SettingsStates: () => ({ workflowStates: STATES }),
});

async function open(section: TeamSection) {
  await renderWithApp(
    <TeamSettingsPage ws="acme" teamKey="ENG" section={section} />,
    newClient(),
    `/acme/settings/teams/ENG/${section}`,
  );
}

describe("EstimatesSection", () => {
  it("loads the current scale and saves only changed fields", async () => {
    const gql = mockGraphql(baseHandlers());
    await open("estimates");
    const user = userEvent.setup();
    const scale = await screen.findByLabelText("Estimation scale");
    expect(scale).toHaveValue("fibonacci");
    expect(screen.getByRole("option", { name: "T-shirt sizes (XS, S, M, L, XL)" })).toBeVisible();
    await user.selectOptions(scale, "tShirt");
    await user.click(screen.getByLabelText("Allow zero estimates"));
    await user.selectOptions(screen.getByLabelText("Default estimate"), "M");
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(gql.of("SettingsTeamUpdate")).toHaveLength(1));
    expect(gql.of("SettingsTeamUpdate")[0]!.variables).toEqual({
      id: "team-eng",
      input: {
        issueEstimationType: "tShirt",
        issueEstimationAllowZero: true,
        defaultIssueEstimate: 3,
      },
    });
  });

  it("disables the options for an unused scale", async () => {
    mockGraphql(baseHandlers({ issueEstimationType: "notUsed" }));
    await open("estimates");
    expect(await screen.findByLabelText("Extended estimates")).toBeDisabled();
    expect(screen.getByLabelText("Default estimate")).toBeDisabled();
  });

  it("shows the server error", async () => {
    mockGraphql(baseHandlers({}, () => new Error("estimate scale is invalid")));
    await open("estimates");
    const user = userEvent.setup();
    await user.click(await screen.findByLabelText("Extended estimates"));
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("estimate scale is invalid");
  });

  it("is read-only with a note when inherited from the parent team", async () => {
    const gql = mockGraphql(baseHandlers({ parentId: "team-p", inheritIssueEstimation: true }));
    await open("estimates");
    expect(await screen.findByRole("note")).toHaveTextContent("inherited from the parent team");
    expect(screen.getByLabelText("Estimation scale")).toBeDisabled();
    expect(screen.getByLabelText("Allow zero estimates")).toBeDisabled();
    expect(screen.queryByRole("button", { name: "Save" })).not.toBeInTheDocument();
    expect(gql.of("SettingsTeamUpdate")).toHaveLength(0);
  });
});

describe("AutomationsSection", () => {
  it("loads values and saves auto-close with a canceled status", async () => {
    const gql = mockGraphql(baseHandlers());
    await open("automations");
    const user = userEvent.setup();
    const period = await screen.findByLabelText("Auto-close period");
    expect(period).toHaveValue("0");
    expect(screen.getByLabelText("Auto-close status")).toBeDisabled();
    expect(screen.getByLabelText("Auto-archive period")).toHaveValue("6");
    await user.selectOptions(period, "3");
    const status = screen.getByLabelText("Auto-close status");
    expect(status).toBeEnabled();
    // Nur Status der Kategorie canceled stehen zur Wahl.
    await screen.findByRole("option", { name: "Canceled" });
    expect(screen.queryByRole("option", { name: "In Progress" })).not.toBeInTheDocument();
    await user.selectOptions(status, "s-canceled");
    await user.selectOptions(screen.getByLabelText("Auto-archive period"), "12");
    await user.click(screen.getByLabelText("Auto-close parent issues"));
    await user.click(screen.getByLabelText("Auto-close sub-issues"));
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(gql.of("SettingsTeamUpdate")).toHaveLength(1));
    expect(gql.of("SettingsTeamUpdate")[0]!.variables).toEqual({
      id: "team-eng",
      input: {
        autoClosePeriod: 3,
        autoCloseStateId: "s-canceled",
        autoArchivePeriod: 12,
        autoCloseParentIssues: true,
        autoCloseChildIssues: true,
      },
    });
  });

  it("switches auto-close off with 0", async () => {
    const gql = mockGraphql(baseHandlers({ autoClosePeriod: 6, autoCloseStateId: "s-canceled" }));
    await open("automations");
    const user = userEvent.setup();
    const period = await screen.findByLabelText("Auto-close period");
    expect(period).toHaveValue("6");
    await user.selectOptions(period, "0");
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(gql.of("SettingsTeamUpdate")).toHaveLength(1));
    expect(gql.of("SettingsTeamUpdate")[0]!.variables).toEqual({
      id: "team-eng",
      input: { autoClosePeriod: 0 },
    });
  });

  it("shows the server error", async () => {
    mockGraphql(baseHandlers({}, () => new Error("must be a canceled status")));
    await open("automations");
    const user = userEvent.setup();
    await user.click(await screen.findByLabelText("Auto-close parent issues"));
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("must be a canceled status");
  });
});
