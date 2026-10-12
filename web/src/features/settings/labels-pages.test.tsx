import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { mockGraphql, newClient, renderWithApp, TEAM_META } from "@/features/issues/test-fixtures";
import type { IssueLabel } from "@/gql/graphql";

import { TeamSettingsPage } from "./settings-pages";
import { workspaceSettingsNav } from "./nav";
import { WorkspaceLabelsPage } from "./workspace-config-pages";

afterEach(() => vi.unstubAllGlobals());

function label(over: Partial<IssueLabel> & Pick<IssueLabel, "id" | "name">): IssueLabel {
  return { ...TEAM_META.labels[0]!, teamId: null, ...over };
}

const WS_LABELS = [
  label({ id: "g-area", name: "Area", isGroup: true }),
  label({ id: "l-web", name: "Web", parentId: "g-area", color: "#4cb782" }),
  label({ id: "l-prio", name: "Customer" }),
  label({ id: "l-old", name: "Legacy", retiredAt: "2026-02-01T00:00:00Z" }),
  label({ id: "l-team", name: "Team only", teamId: "team-eng" }),
];

const handlers = (over: Record<string, (v: Record<string, unknown>) => unknown> = {}) => ({
  SettingsWorkspaceLabels: () => ({ issueLabels: WS_LABELS }),
  SettingsLabelCreate: () => ({ issueLabelCreate: { success: true } }),
  SettingsLabelUpdate: () => ({ issueLabelUpdate: { success: true } }),
  SettingsLabelRetire: () => ({ issueLabelRetire: { success: true } }),
  SettingsLabelRestore: () => ({ issueLabelRestore: { success: true } }),
  SettingsTeam: () => ({ team: TEAM_META.team }),
  SettingsLabels: () => ({ issueLabels: TEAM_META.labels }),
  ...over,
});

async function openWs() {
  await renderWithApp(<WorkspaceLabelsPage ws="acme" />, newClient(), "/acme/settings/labels");
}

describe("workspace settings navigation", () => {
  it("lists the Labels and Templates pages", () => {
    const nav = workspaceSettingsNav("acme", null, "labels");
    expect(nav.find((n) => n.label === "Labels")).toMatchObject({
      to: "/acme/settings/labels",
      active: true,
    });
    expect(nav.find((n) => n.label === "Templates")).toMatchObject({
      to: "/acme/settings/templates",
      active: false,
    });
  });
});

describe("WorkspaceLabelsPage", () => {
  it("shows workspace labels with groups, retired ones separately, and no team labels", async () => {
    mockGraphql(handlers());
    await openWs();
    expect(await screen.findByText("Customer")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Retire Area" })).toBeInTheDocument();
    expect(screen.getByText("Web")).toBeInTheDocument();
    expect(screen.queryByText("Team only")).toBeNull();
    const retired = screen.getByText("Retired").parentElement!;
    expect(within(retired).getByText("Legacy")).toBeInTheDocument();
  });

  it("creates a label, a group and a label inside a group", async () => {
    const gql = mockGraphql(handlers());
    await openWs();
    const user = userEvent.setup();
    await screen.findByText("Customer");
    await user.type(screen.getByLabelText("New label name"), "Security");
    await user.click(screen.getByRole("button", { name: /Create/ }));
    await waitFor(() => expect(gql.of("SettingsLabelCreate")).toHaveLength(1));
    // Workspace-Label: kein teamId.
    expect(gql.of("SettingsLabelCreate")[0]!.variables).toEqual({
      input: { name: "Security", color: "#5e6ad2" },
    });
    await user.type(screen.getByLabelText("New label name"), "Platform");
    await user.selectOptions(screen.getByLabelText("Label group"), "g-area");
    await user.click(screen.getByRole("button", { name: /Create/ }));
    await waitFor(() => expect(gql.of("SettingsLabelCreate")).toHaveLength(2));
    expect(gql.of("SettingsLabelCreate")[1]!.variables).toEqual({
      input: { name: "Platform", color: "#5e6ad2", parentId: "g-area" },
    });
    await user.type(screen.getByLabelText("New label name"), "Topics");
    await user.click(screen.getByRole("checkbox", { name: "Group" }));
    await user.click(screen.getByRole("button", { name: /Create/ }));
    await waitFor(() => expect(gql.of("SettingsLabelCreate")).toHaveLength(3));
    expect(gql.of("SettingsLabelCreate")[2]!.variables).toEqual({
      input: { name: "Topics", color: "#5e6ad2", isGroup: true },
    });
  });

  it("rejects an invalid color before sending", async () => {
    const gql = mockGraphql(handlers());
    await openWs();
    const user = userEvent.setup();
    await screen.findByText("Customer");
    await user.type(screen.getByLabelText("New label name"), "Bad");
    await user.clear(screen.getByLabelText("Label color"));
    await user.type(screen.getByLabelText("Label color"), "red");
    expect(screen.getByRole("button", { name: /Create/ })).toBeDisabled();
    expect(gql.of("SettingsLabelCreate")).toHaveLength(0);
  });

  it("renames, recolors, retires and restores", async () => {
    const gql = mockGraphql(handlers());
    await openWs();
    const user = userEvent.setup();
    await screen.findByText("Customer");
    await user.click(screen.getByRole("button", { name: "Rename Customer" }));
    const form = screen.getByRole("form", { name: "Edit Customer" });
    await user.clear(within(form).getByLabelText("Label name"));
    await user.type(within(form).getByLabelText("Label name"), "Client");
    await user.clear(within(form).getByLabelText("Label color"));
    await user.type(within(form).getByLabelText("Label color"), "#112233");
    await user.click(within(form).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(gql.of("SettingsLabelUpdate")).toHaveLength(1));
    expect(gql.of("SettingsLabelUpdate")[0]!.variables).toEqual({
      id: "l-prio",
      input: { name: "Client", color: "#112233" },
    });
    await user.click(screen.getByRole("button", { name: "Retire Web" }));
    await waitFor(() => expect(gql.of("SettingsLabelRetire")).toHaveLength(1));
    expect(gql.of("SettingsLabelRetire")[0]!.variables).toEqual({ id: "l-web" });
    await user.click(screen.getByRole("button", { name: "Restore Legacy" }));
    await waitFor(() => expect(gql.of("SettingsLabelRestore")).toHaveLength(1));
    expect(gql.of("SettingsLabelRestore")[0]!.variables).toEqual({ id: "l-old" });
  });

  it("shows the server error", async () => {
    mockGraphql(handlers({ SettingsLabelCreate: () => new Error("label name already exists") }));
    await openWs();
    const user = userEvent.setup();
    await screen.findByText("Customer");
    await user.type(screen.getByLabelText("New label name"), "Customer");
    await user.click(screen.getByRole("button", { name: /Create/ }));
    expect(await screen.findByRole("alert")).toHaveTextContent("label name already exists");
  });

  it("shows a load error", async () => {
    mockGraphql(handlers({ SettingsWorkspaceLabels: () => new Error("boom") }));
    await openWs();
    expect(await screen.findByText("Could not load labels")).toBeInTheDocument();
  });
});

describe("Team issue labels (shared manager)", () => {
  it("creates labels with the team id", async () => {
    const gql = mockGraphql(handlers());
    await renderWithApp(
      <TeamSettingsPage ws="acme" teamKey="ENG" section="issue-labels" />,
      newClient(),
      "/acme/settings/teams/ENG/issue-labels",
    );
    const user = userEvent.setup();
    expect(await screen.findByText("Bug")).toBeInTheDocument();
    await user.type(screen.getByLabelText("New label name"), "Regression");
    await user.click(screen.getByRole("button", { name: /Create/ }));
    await waitFor(() => expect(gql.of("SettingsLabelCreate")).toHaveLength(1));
    expect(gql.of("SettingsLabelCreate")[0]!.variables).toEqual({
      input: { name: "Regression", color: "#5e6ad2", teamId: "team-eng" },
    });
  });
});
