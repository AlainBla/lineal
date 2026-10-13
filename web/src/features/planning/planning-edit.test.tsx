import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";
import {
  makeIssue,
  mockGraphql,
  newClient,
  renderWithApp,
  TEAM_META,
} from "@/features/issues/test-fixtures";
import type { Initiative, Project } from "@/gql/graphql";
import { attachShortcuts, shortcuts } from "@/lib/shortcuts";

import {
  closeCreateInitiative,
  closeCreateProject,
  openCreateProject,
  PlanningDialogHost,
} from "./create-dialogs";
import { InitiativeDetail } from "./initiative-detail";
import { ProjectPage } from "./project-page";

// Echtzeit-Endpunkt nicht erreichbar: die Beschreibung fällt auf Stufe 1 (Autosave) zurück.
vi.mock("@/features/collab/use-collab", () => ({
  useCollab: () => ({
    provider: null,
    status: "fallback",
    canWrite: false,
    synced: false,
    everConnected: false,
    generation: 0,
  }),
}));

const ref = (id: string, name: string, key: string | null = null) => ({ id, name, key });

const project = (over: Partial<Project> = {}): Project =>
  ({
    id: "p-1",
    identifier: "P-ENG-1",
    previousIdentifiers: [],
    slugId: "abc",
    name: "Checkout Relaunch",
    summary: null,
    description: null,
    icon: null,
    color: "#4cb782",
    priority: 0,
    status: { id: "ps-1", name: "Backlog", type: "backlog", color: "#bbb" },
    health: null,
    progress: 0,
    scope: 0,
    completedScope: 0,
    issueCount: 2,
    completedIssueCount: 0,
    startDate: null,
    startDateResolution: null,
    targetDate: null,
    targetDateResolution: null,
    startedAt: null,
    completedAt: null,
    canceledAt: null,
    archivedAt: null,
    leadTeam: ref("team-eng", "Engineering", "ENG"),
    teams: [ref("team-eng", "Engineering", "ENG")],
    lead: null,
    members: [],
    labels: [],
    initiatives: [],
    milestones: [
      {
        id: "m-alpha",
        project: { id: "p-1", identifier: "P-ENG-1", name: "Checkout Relaunch" },
        name: "Alpha",
        description: null,
        targetDate: "2026-11-30",
        sortOrder: 1,
        status: "planned",
        progress: 0,
        issueCount: 1,
        completedIssueCount: 0,
        createdAt: "2026-10-01T00:00:00Z",
        updatedAt: "2026-10-01T00:00:00Z",
      },
    ],
    openIssueCount: null,
    createdAt: "2026-10-01T00:00:00Z",
    updatedAt: "2026-10-01T00:00:00Z",
    ...over,
  }) as Project;

const initiative = (): Initiative =>
  ({
    id: "i-1",
    identifier: "I-1",
    slugId: "abc",
    name: "Growth",
    summary: null,
    description: null,
    color: null,
    icon: null,
    status: "Active",
    statusType: "started",
    priority: 0,
    health: null,
    targetDate: null,
    targetDateResolution: null,
    startedAt: null,
    completedAt: null,
    owner: null,
    leadTeam: null,
    visibility: "public",
    parentInitiatives: [],
    subInitiatives: [ref("i-2", "Activation")],
    labels: [],
    projects: [],
    healthRollup: { color: null, onTrack: 0, atRisk: 0, offTrack: 0, none: 0 },
    archivedAt: null,
    createdAt: "2026-10-01T00:00:00Z",
    updatedAt: "2026-10-01T00:00:00Z",
  }) as Initiative;

const TEAMS = {
  teams: [
    {
      id: "team-eng",
      key: "ENG",
      name: "Engineering",
      parentId: null,
      issueEstimationType: "notUsed",
    },
    { id: "team-des", key: "DES", name: "Design", parentId: null, issueEstimationType: "notUsed" },
  ],
};
const USERS = {
  users: [
    { id: "u-alice", name: "alice", displayName: "Alice" },
    { id: "u-bob", name: "bob", displayName: "Bob" },
  ],
};

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  shortcuts.reset();
  closeCreateProject();
  closeCreateInitiative();
});

describe("Projekt bearbeiten (F.01)", () => {
  const base = (current: () => Project) => ({
    PlanningProject: () => ({ project: current() }),
    PlanningProjectUpdates: () => ({ projectUpdates: [] }),
    IssueTeams: () => TEAMS,
    KnownUsers: () => USERS,
    PlanningProjectLabels: () => ({
      projectLabels: [{ id: "l-1", name: "Mobile", color: "#fff" }],
    }),
    PlanningProjectUpdate: () => ({ projectUpdate: { success: true } }),
  });

  it("schickt Status, Lead, Team, Mitglieder, Datum und Name als projectUpdate", async () => {
    const gql = mockGraphql(base(() => project()));
    await renderWithApp(
      <ProjectPage ws="acme" projectRef="P-ENG-1" tab="overview" />,
      newClient(),
      "/acme/project/P-ENG-1/overview",
    );
    const user = userEvent.setup();
    await screen.findByLabelText("Project name");
    const sent = () => gql.of("PlanningProjectUpdate").map((c) => c.variables.input);

    await user.selectOptions(screen.getByLabelText("Status"), "paused");
    await waitFor(() => expect(sent()).toContainEqual({ state: "paused" }));
    await user.selectOptions(await screen.findByRole("combobox", { name: "Lead" }), "u-bob");
    await waitFor(() => expect(sent()).toContainEqual({ lead: "u-bob" }));
    await user.selectOptions(screen.getByLabelText("Add team…"), "team-des");
    await waitFor(() => expect(sent()).toContainEqual({ addTeams: ["team-des"] }));
    await user.selectOptions(screen.getByLabelText("Add member…"), "u-alice");
    await waitFor(() => expect(sent()).toContainEqual({ addMembers: ["u-alice"] }));
    await user.selectOptions(screen.getByLabelText("Add label…"), "l-1");
    await waitFor(() => expect(sent()).toContainEqual({ labels: ["l-1"] }));
    await user.selectOptions(screen.getByLabelText("Priority"), "2");
    await waitFor(() => expect(sent()).toContainEqual({ priority: 2 }));

    const target = screen.getByLabelText("Target date");
    await user.type(target, "2026-12-31");
    await waitFor(() =>
      expect(sent()).toContainEqual({ targetDate: "2026-12-31", targetDateResolution: null }),
    );

    const name = screen.getByLabelText("Project name");
    await user.clear(name);
    await user.type(name, "Checkout 2{Enter}");
    await waitFor(() => expect(sent()).toContainEqual({ name: "Checkout 2" }));
  });

  it("zeigt Fehler des Servers (Rechte/Regeln) statt sie clientseitig zu verhindern", async () => {
    const forbidden = Object.assign(new Error("Only admins and the lead can do this."), {
      code: "forbidden",
    });
    mockGraphql({
      ...base(() => project()),
      PlanningProjectUpdate: () => forbidden,
    });
    await renderWithApp(
      <ProjectPage ws="acme" projectRef="P-ENG-1" tab="overview" />,
      newClient(),
      "/acme/project/P-ENG-1/overview",
    );
    const user = userEvent.setup();
    await user.selectOptions(await screen.findByLabelText("Status"), "paused");
    expect(await screen.findByRole("alert")).toHaveTextContent(/permission/i);
  });

  it("Meilensteine: umbenennen, Datum ändern, löschen nur nach Bestätigung", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValueOnce(false).mockReturnValue(true);
    const gql = mockGraphql({
      ...base(() => project()),
      PlanningMilestoneUpdate: () => ({ projectMilestoneUpdate: { success: true } }),
      PlanningMilestoneDelete: () => ({ projectMilestoneDelete: { success: true } }),
    });
    await renderWithApp(
      <ProjectPage ws="acme" projectRef="P-ENG-1" tab="milestones" />,
      newClient(),
      "/acme/project/P-ENG-1/milestones",
    );
    const user = userEvent.setup();
    const field = await screen.findByLabelText("Milestone name Alpha");
    await user.clear(field);
    await user.type(field, "Alpha 2{Enter}");
    await waitFor(() => expect(gql.of("PlanningMilestoneUpdate")).toHaveLength(1));
    expect(gql.of("PlanningMilestoneUpdate")[0]!.variables).toMatchObject({
      id: "m-alpha",
      input: { name: "Alpha 2" },
    });
    const date = screen.getByLabelText("Target date of Alpha");
    fireEvent.change(date, { target: { value: "2026-12-15" } });
    await waitFor(() =>
      expect(gql.of("PlanningMilestoneUpdate").at(-1)!.variables).toMatchObject({
        input: { targetDate: "2026-12-15" },
      }),
    );
    await user.click(screen.getByRole("button", { name: "Delete milestone Alpha" }));
    expect(gql.of("PlanningMilestoneDelete")).toHaveLength(0);
    await user.click(screen.getByRole("button", { name: "Delete milestone Alpha" }));
    await waitFor(() => expect(gql.of("PlanningMilestoneDelete")).toHaveLength(1));
    expect(confirm).toHaveBeenCalledTimes(2);
  });

  it("Archivieren und Löschen fragen nach", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValueOnce(false).mockReturnValue(true);
    const gql = mockGraphql({
      ...base(() => project()),
      PlanningProjectArchive: () => ({ projectArchive: { success: true } }),
      PlanningProjectDelete: () => ({ projectDelete: { success: true } }),
    });
    await renderWithApp(
      <ProjectPage ws="acme" projectRef="P-ENG-1" tab="overview" />,
      newClient(),
      "/acme/project/P-ENG-1/overview",
    );
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Archive" }));
    expect(gql.of("PlanningProjectArchive")).toHaveLength(0);
    await user.click(screen.getByRole("button", { name: "Archive" }));
    await waitFor(() => expect(gql.of("PlanningProjectArchive")).toHaveLength(1));
    expect(gql.of("PlanningProjectArchive")[0]!.variables).toEqual({ id: "p-1", archived: true });
    await user.click(screen.getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(gql.of("PlanningProjectDelete")).toHaveLength(1));
    expect(confirm).toHaveBeenCalledTimes(3);
  });

  it("Issues-Tab gruppiert nach Meilenstein, danach „No milestone“", async () => {
    const issues = [
      makeIssue({
        id: "a",
        identifier: "ENG-1",
        title: "Im Alpha",
        projectId: "p-1",
        projectMilestoneId: "m-alpha",
      }),
      makeIssue({ id: "b", identifier: "ENG-2", title: "Ohne", projectId: "p-1" }),
    ];
    mockGraphql({
      ...base(() => project()),
      PlanningScopedIssues: () => ({
        issues: { nodes: issues, hasNextPage: false, endCursor: null },
      }),
      TeamMeta: () => ({
        team: TEAM_META.team,
        workflowStates: TEAM_META.states,
        issueLabels: TEAM_META.labels,
        cycles: [],
        projects: TEAM_META.projects,
      }),
    });
    await renderWithApp(
      <ProjectPage ws="acme" projectRef="P-ENG-1" tab="issues" />,
      newClient(),
      "/acme/project/P-ENG-1/issues",
    );
    const alpha = await screen.findByRole("region", { name: "Milestone Alpha" });
    expect(within(alpha).getByText("Im Alpha")).toBeInTheDocument();
    const none = screen.getByRole("region", { name: "Milestone No milestone" });
    expect(within(none).getByText("Ohne")).toBeInTheDocument();
  });
});

describe("Initiative bearbeiten (F.01)", () => {
  it("Owner, Status, Projekt zuordnen, Sub-Initiative entfernen, Löschen mit Bestätigung", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const gql = mockGraphql({
      PlanningInitiative: () => ({ initiative: initiative() }),
      PlanningInitiativeUpdate: () => ({ initiativeUpdate: { success: true } }),
      PlanningProjects: () => ({
        projectsPage: {
          nodes: [project()],
          pageInfo: { hasNextPage: false, endCursor: null },
        },
      }),
      PlanningInitiativeProjectAdd: () => ({ initiativeToProjectCreate: { success: true } }),
      PlanningSubInitiativeRemove: () => ({ initiativeParentRemove: { success: true } }),
      PlanningInitiativeDelete: () => ({ initiativeDelete: { success: true } }),
      IssueTeams: () => TEAMS,
      KnownUsers: () => USERS,
    });
    const user = userEvent.setup();
    const { unmount } = await renderWithApp(
      <InitiativeDetail ws="acme" initiativeRef="I-1" tab="overview" />,
      newClient(),
      "/acme/initiative/I-1/overview",
    );
    const owner = await screen.findByRole("combobox", { name: "Owner" });
    await waitFor(() => expect(within(owner).getAllByRole("option").length).toBeGreaterThan(1));
    await user.selectOptions(owner, "u-bob");
    await waitFor(() =>
      expect(gql.of("PlanningInitiativeUpdate").map((c) => c.variables)).toContainEqual({
        id: "i-1",
        input: { owner: "u-bob" },
      }),
    );
    await user.selectOptions(screen.getByLabelText("Status"), "completed");
    await waitFor(() =>
      expect(gql.of("PlanningInitiativeUpdate").map((c) => c.variables.input)).toContainEqual({
        status: "completed",
      }),
    );
    await user.click(screen.getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(gql.of("PlanningInitiativeDelete")).toHaveLength(1));
    expect(confirm).toHaveBeenCalled();
    unmount();

    await renderWithApp(
      <InitiativeDetail ws="acme" initiativeRef="I-1" tab="projects" />,
      newClient(),
      "/acme/initiative/I-1/projects",
    );
    const addProject = await screen.findByLabelText("Add project…");
    await waitFor(() =>
      expect(within(addProject).getAllByRole("option").length).toBeGreaterThan(1),
    );
    await user.selectOptions(addProject, "p-1");
    await waitFor(() => expect(gql.of("PlanningInitiativeProjectAdd")).toHaveLength(1));
    expect(gql.of("PlanningInitiativeProjectAdd")[0]!.variables).toEqual({
      initiativeId: "i-1",
      projectId: "p-1",
    });
  });

  it("Sub-Initiative entfernen sendet initiativeParentRemove mit Kind und Eltern", async () => {
    const gql = mockGraphql({
      PlanningInitiative: () => ({ initiative: initiative() }),
      PlanningInitiatives: () => ({ initiatives: [initiative()] }),
      PlanningSubInitiativeRemove: () => ({ initiativeParentRemove: { success: true } }),
    });
    await renderWithApp(
      <InitiativeDetail ws="acme" initiativeRef="I-1" tab="sub-initiatives" />,
      newClient(),
      "/acme/initiative/I-1/sub-initiatives",
    );
    const user = userEvent.setup();
    await user.click(
      await screen.findByRole("button", { name: "Remove Activation from sub-initiatives" }),
    );
    await waitFor(() => expect(gql.of("PlanningSubInitiativeRemove")).toHaveLength(1));
    expect(gql.of("PlanningSubInitiativeRemove")[0]!.variables).toEqual({
      initiativeId: "i-2",
      parentId: "i-1",
    });
  });
});

describe("New project / New initiative (F.01)", () => {
  it("N P öffnet den Dialog mit Team aus dem Pfad; Anlegen sendet Teams, Lead-Team und Status", async () => {
    const gql = mockGraphql({
      IssueTeams: () => TEAMS,
      KnownUsers: () => USERS,
      PlanningProjectCreate: () => ({
        projectCreate: {
          success: true,
          project: { id: "p-9", identifier: "P-ENG-9", name: "Neu" },
        },
      }),
    });
    await renderWithApp(<PlanningDialogHost />, newClient(), "/acme/team/ENG/projects/all");
    const user = userEvent.setup();
    act(() => openCreateProject({ teamKey: "ENG" }));
    const name = await screen.findByLabelText("Project name");
    await waitFor(() =>
      expect(screen.getByRole("checkbox", { name: /Engineering/ })).toBeChecked(),
    );
    expect(screen.getByRole("checkbox", { name: /Design/ })).not.toBeChecked();
    expect(screen.getByRole("button", { name: "Create project" })).toBeDisabled();
    await user.type(name, "Neu");
    await user.click(screen.getByRole("checkbox", { name: /Design/ }));
    await user.selectOptions(screen.getByLabelText("Status"), "planned");
    await user.click(screen.getByRole("button", { name: "Create project" }));
    await waitFor(() => expect(gql.of("PlanningProjectCreate")).toHaveLength(1));
    expect(gql.of("PlanningProjectCreate")[0]!.variables).toEqual({
      input: {
        name: "Neu",
        addTeams: ["team-eng", "team-des"],
        leadTeam: "team-eng",
        state: "planned",
      },
    });
  });

  it("zeigt den Server-Fehler im Dialog und bleibt offen", async () => {
    mockGraphql({
      IssueTeams: () => TEAMS,
      KnownUsers: () => USERS,
      PlanningProjectCreate: () => new Error("Could not find team"),
    });
    await renderWithApp(<PlanningDialogHost />, newClient());
    const user = userEvent.setup();
    act(() => openCreateProject({ teamKey: "ENG" }));
    await user.type(await screen.findByLabelText("Project name"), "X");
    await waitFor(() =>
      expect(screen.getByRole("checkbox", { name: /Engineering/ })).toBeChecked(),
    );
    await user.click(screen.getByRole("button", { name: "Create project" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/could not find team/i);
    expect(screen.getByLabelText("Project name")).toBeInTheDocument();
  });

  it("N I öffnet den Initiativen-Dialog", async () => {
    const gql = mockGraphql({
      IssueTeams: () => TEAMS,
      KnownUsers: () => USERS,
      PlanningInitiativeCreate: () => ({
        initiativeCreate: {
          success: true,
          initiative: { id: "i-9", identifier: "I-9", name: "Ziel" },
        },
      }),
    });
    const detach = attachShortcuts();
    await renderWithApp(<PlanningDialogHost />, newClient());
    const user = userEvent.setup();
    await user.keyboard("n");
    await user.keyboard("i");
    const name = await screen.findByLabelText("Initiative name");
    await user.type(name, "Ziel");
    await user.click(screen.getByRole("button", { name: "Create initiative" }));
    await waitFor(() => expect(gql.of("PlanningInitiativeCreate")).toHaveLength(1));
    expect(gql.of("PlanningInitiativeCreate")[0]!.variables).toEqual({
      input: { name: "Ziel", status: "planned" },
    });
    detach();
  });
});
