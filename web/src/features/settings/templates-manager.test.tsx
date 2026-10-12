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
import type { Template } from "@/gql/graphql";

import { TeamSettingsPage } from "./settings-pages";
import { WorkspaceTemplatesPage } from "./workspace-config-pages";

afterEach(() => vi.unstubAllGlobals());

function tpl(over: Partial<Template> & Pick<Template, "id" | "name" | "type">): Template {
  return {
    description: null,
    icon: null,
    color: null,
    teamId: "team-eng",
    teamKey: "ENG",
    templateData: {},
    sortOrder: 0,
    inheritedFromId: null,
    isForm: false,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
    ...over,
  };
}

const BUG = tpl({
  id: "t-bug",
  name: "Bug report",
  type: "issue",
  description: "For defects",
  templateData: {
    title: "Bug: ",
    description: "## Steps",
    priority: 2,
    stateId: "s-todo",
    labelIds: ["l-bug"],
    subIssues: [{ title: "Repro" }],
  },
});
const INHERITED = tpl({
  id: "t-parent",
  name: "Parent template",
  type: "issue",
  teamId: "team-parent",
  teamKey: "PAR",
});
const WORKSPACE = tpl({
  id: "t-ws",
  name: "Workspace spec",
  type: "document",
  teamId: null,
  teamKey: null,
});
const PROJECT = tpl({ id: "t-proj", name: "Launch", type: "project" });

const LABELS = [
  ...TEAM_META.labels,
  { ...TEAM_META.labels[0]!, id: "l-ws", name: "Customer", teamId: null },
];

const handlers = (over: Record<string, (v: Record<string, unknown>) => unknown> = {}) => ({
  SettingsTeam: () => ({ team: TEAM_META.team }),
  SettingsTemplates: (v: Record<string, unknown>) => ({
    templates: v.teamId ? [BUG, INHERITED, WORKSPACE, PROJECT] : [WORKSPACE, BUG],
  }),
  SettingsTemplateDefaults: () => ({
    teamDefaultTemplates: { forMembersId: "t-bug", forNonMembersId: null, projectId: null },
  }),
  SettingsLabels: () => ({ issueLabels: LABELS }),
  SettingsStates: () => ({ workflowStates: STATES }),
  SettingsTeamMembers: () => ({ users: [{ id: "user-bob", name: "Bob", displayName: "Bob" }] }),
  SettingsWorkspaceLabels: () => ({ issueLabels: LABELS }),
  SettingsTemplateCreate: () => ({ templateCreate: { success: true } }),
  SettingsTemplateUpdate: () => ({ templateUpdate: { success: true } }),
  SettingsTemplateDelete: () => ({ templateDelete: { success: true } }),
  SettingsTemplateDefaultSet: () => ({ teamDefaultTemplateSet: true }),
  ...over,
});

async function openTeam() {
  await renderWithApp(
    <TeamSettingsPage ws="acme" teamKey="ENG" section="templates" />,
    newClient(),
    "/acme/settings/teams/ENG/templates",
  );
}

describe("Team templates section", () => {
  it("lists templates by type with origin and default markers", async () => {
    mockGraphql(handlers());
    await openTeam();
    const issues = await screen.findByRole("group", { name: "Issue templates" });
    expect(within(issues).getByText("Bug report")).toBeInTheDocument();
    expect(within(issues).getByText("Default for members")).toBeInTheDocument();
    expect(within(issues).getByText("Inherited from PAR")).toBeInTheDocument();
    // Geerbte Templates sind schreibgeschützt.
    expect(within(issues).queryByRole("button", { name: "Edit Parent template" })).toBeNull();
    expect(within(issues).getByRole("button", { name: "Edit Bug report" })).toBeInTheDocument();
    const docs = screen.getByRole("group", { name: "Document templates" });
    expect(within(docs).getByText("Workspace template")).toBeInTheDocument();
    expect(within(docs).queryByRole("button", { name: /Edit/ })).toBeNull();
    expect(screen.getByRole("group", { name: "Project templates" })).toHaveTextContent("Launch");
  });

  it("creates an issue template with preset fields", async () => {
    const gql = mockGraphql(handlers());
    await openTeam();
    const user = userEvent.setup();
    await screen.findByRole("group", { name: "Issue templates" });
    await user.click(screen.getByRole("button", { name: /New template/ }));
    await user.type(screen.getByLabelText("Template name"), "Feature");
    await user.type(screen.getByLabelText("Title"), "Feature: ");
    await user.type(screen.getByLabelText("Body"), "## Goal");
    await user.selectOptions(screen.getByLabelText("Priority"), "3");
    await user.selectOptions(screen.getByLabelText("Status"), "s-progress");
    await user.selectOptions(screen.getByLabelText("Assignee"), "creator");
    await user.type(screen.getByLabelText("Estimate"), "5");
    await user.click(screen.getByLabelText("Label Bug"));
    await user.click(screen.getByRole("button", { name: "Save template" }));
    await waitFor(() => expect(gql.of("SettingsTemplateCreate")).toHaveLength(1));
    expect(gql.of("SettingsTemplateCreate")[0]!.variables).toEqual({
      input: {
        type: "issue",
        name: "Feature",
        teamId: "team-eng",
        templateData: {
          title: "Feature: ",
          description: "## Goal",
          priority: 3,
          stateId: "s-progress",
          assigneeId: "creator",
          estimate: 5,
          labelIds: ["l-bug"],
        },
      },
    });
  });

  it("edits a template and keeps fields the editor does not know", async () => {
    const gql = mockGraphql(handlers());
    await openTeam();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Edit Bug report" }));
    expect(screen.getByLabelText("Title")).toHaveValue("Bug: ");
    expect(screen.getByLabelText("Label Bug")).toBeChecked();
    await user.clear(screen.getByLabelText("Template name"));
    await user.type(screen.getByLabelText("Template name"), "Defect");
    await user.selectOptions(screen.getByLabelText("Priority"), "");
    await user.click(screen.getByRole("button", { name: "Save template" }));
    await waitFor(() => expect(gql.of("SettingsTemplateUpdate")).toHaveLength(1));
    expect(gql.of("SettingsTemplateUpdate")[0]!.variables).toEqual({
      id: "t-bug",
      input: {
        name: "Defect",
        description: "For defects",
        templateData: {
          title: "Bug: ",
          description: "## Steps",
          stateId: "s-todo",
          labelIds: ["l-bug"],
          subIssues: [{ title: "Repro" }],
        },
      },
    });
  });

  it("shows the server error when saving fails", async () => {
    mockGraphql(handlers({ SettingsTemplateCreate: () => new Error("name already taken") }));
    await openTeam();
    const user = userEvent.setup();
    await screen.findByRole("group", { name: "Issue templates" });
    await user.click(screen.getByRole("button", { name: /New template/ }));
    await user.type(screen.getByLabelText("Template name"), "Bug report");
    await user.click(screen.getByRole("button", { name: "Save template" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("name already taken");
  });

  it("deletes only after confirmation", async () => {
    const gql = mockGraphql(handlers());
    await openTeam();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Delete Bug report" }));
    expect(gql.of("SettingsTemplateDelete")).toHaveLength(0);
    await user.click(screen.getByRole("button", { name: "Confirm delete Bug report" }));
    await waitFor(() => expect(gql.of("SettingsTemplateDelete")).toHaveLength(1));
    expect(gql.of("SettingsTemplateDelete")[0]!.variables).toEqual({ id: "t-bug" });
  });

  it("sets and clears default templates", async () => {
    const gql = mockGraphql(handlers());
    await openTeam();
    const user = userEvent.setup();
    const nonMembers = await screen.findByLabelText("Default issue template for non-members");
    await user.selectOptions(nonMembers, "t-parent");
    await waitFor(() => expect(gql.of("SettingsTemplateDefaultSet")).toHaveLength(1));
    expect(gql.of("SettingsTemplateDefaultSet")[0]!.variables).toEqual({
      teamId: "team-eng",
      slot: "forNonMembers",
      templateId: "t-parent",
    });
    const members = screen.getByLabelText("Default issue template for team members");
    expect(members).toHaveValue("t-bug");
    await user.selectOptions(members, "");
    await waitFor(() => expect(gql.of("SettingsTemplateDefaultSet")).toHaveLength(2));
    expect(gql.of("SettingsTemplateDefaultSet")[1]!.variables).toEqual({
      teamId: "team-eng",
      slot: "forMembers",
      templateId: null,
    });
    await user.selectOptions(screen.getByLabelText("Default project template"), "t-proj");
    await waitFor(() => expect(gql.of("SettingsTemplateDefaultSet")).toHaveLength(3));
  });

  it("shows a load error", async () => {
    mockGraphql(handlers({ SettingsTemplates: () => new Error("boom") }));
    await openTeam();
    expect(await screen.findByText("Could not load templates")).toBeInTheDocument();
  });
});

describe("WorkspaceTemplatesPage", () => {
  it("lists only workspace templates and creates one without a team", async () => {
    const gql = mockGraphql(handlers());
    await renderWithApp(
      <WorkspaceTemplatesPage ws="acme" />,
      newClient(),
      "/acme/settings/templates",
    );
    const user = userEvent.setup();
    const docs = await screen.findByRole("group", { name: "Document templates" });
    expect(within(docs).getByText("Workspace spec")).toBeInTheDocument();
    expect(screen.queryByText("Default for members")).toBeNull();
    expect(screen.queryByLabelText("Default project template")).toBeNull();
    await user.selectOptions(screen.getByLabelText("New template type"), "document");
    await user.click(screen.getByRole("button", { name: /New template/ }));
    await user.type(screen.getByLabelText("Template name"), "Meeting notes");
    await user.type(screen.getByLabelText("Body"), "Agenda");
    await user.click(screen.getByRole("button", { name: "Save template" }));
    await waitFor(() => expect(gql.of("SettingsTemplateCreate")).toHaveLength(1));
    expect(gql.of("SettingsTemplateCreate")[0]!.variables).toEqual({
      input: { type: "document", name: "Meeting notes", templateData: { content: "Agenda" } },
    });
    expect(gql.of("SettingsTemplates")[0]!.variables).toEqual({ teamId: null });
  });

  it("uses status categories and workspace labels in the issue editor", async () => {
    mockGraphql(handlers());
    await renderWithApp(
      <WorkspaceTemplatesPage ws="acme" />,
      newClient(),
      "/acme/settings/templates",
    );
    const user = userEvent.setup();
    await screen.findByRole("group", { name: "Issue templates" });
    await user.click(screen.getByRole("button", { name: /New template/ }));
    expect(screen.getByRole("option", { name: "Started" })).toBeInTheDocument();
    expect(screen.queryByRole("option", { name: "Duplicate" })).toBeNull();
    expect(await screen.findByLabelText("Label Customer")).toBeInTheDocument();
    expect(screen.queryByLabelText("Label Bug")).toBeNull();
  });
});
