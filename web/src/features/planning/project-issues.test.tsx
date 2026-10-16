import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";
import {
  makeIssue,
  mockGraphql,
  newClient,
  renderWithApp,
  TEAM_META,
} from "@/features/issues/test-fixtures";
import type { Project } from "@/gql/graphql";
import { attachShortcuts, shortcuts } from "@/lib/shortcuts";

import { ProjectPage } from "./project-page";

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
const MS = (id: string, name: string, over: Record<string, unknown> = {}) => ({
  id,
  project: { id: "p-1", identifier: "P-ENG-1", name: "Checkout" },
  name,
  description: null,
  targetDate: "2026-12-01",
  sortOrder: 1,
  status: "next",
  progress: 0.5,
  issueCount: 2,
  completedIssueCount: 1,
  createdAt: "2026-01-01T00:00:00Z",
  updatedAt: "2026-01-01T00:00:00Z",
  ...over,
});

const PROJECT = {
  id: "p-1",
  identifier: "P-ENG-1",
  previousIdentifiers: [],
  slugId: "abc",
  name: "Checkout",
  summary: null,
  description: null,
  icon: null,
  color: "#4cb782",
  priority: 2,
  status: { id: "ps-1", name: "In Progress", type: "started", color: "#f2c94c" },
  health: null,
  progress: 0.5,
  scope: 3,
  completedScope: 1,
  issueCount: 3,
  completedIssueCount: 1,
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
  milestones: [MS("m-1", "Beta"), MS("m-2", "Launch", { targetDate: null, issueCount: 1 })],
} as unknown as Project;

const ISSUES = [
  makeIssue({
    id: "a",
    identifier: "ENG-1",
    title: "Erstes",
    projectId: "p-1",
    projectMilestoneId: "m-1",
  }),
  makeIssue({
    id: "b",
    identifier: "ENG-2",
    title: "Zweites",
    projectId: "p-1",
    projectMilestoneId: "m-1",
  }),
  makeIssue({ id: "c", identifier: "ENG-3", title: "Drittes", projectId: "p-1" }),
];

let detach: () => void;
beforeEach(() => {
  localStorage.clear();
  detach = attachShortcuts();
});
afterEach(() => {
  detach();
  shortcuts.reset();
});

const handlers = (log: Array<Record<string, unknown>> = []) => ({
  PlanningProject: () => ({ project: PROJECT }),
  PlanningProjectUpdates: () => ({ projectUpdates: [] }),
  TeamMeta: () => ({
    team: TEAM_META.team,
    workflowStates: TEAM_META.states,
    issueLabels: [],
    cycles: [],
    projects: [
      {
        id: "p-1",
        identifier: "P-ENG-1",
        name: "Checkout",
        color: "#4cb782",
        milestones: [
          { id: "m-1", name: "Beta", targetDate: "2026-12-01", sortOrder: 1 },
          { id: "m-2", name: "Launch", targetDate: null, sortOrder: 2 },
        ],
      },
    ],
  }),
  ViewIssuesList: () => ({ viewIssues: { nodes: ISSUES, hasNextPage: false, endCursor: null } }),
  ViewPrefs: () => ({
    viewPreferences: { values: { grouping: "milestone" }, hasPersonal: false, hasDefault: false },
  }),
  ViewPrefsUpdate: (v: Record<string, unknown>) => {
    log.push(v);
    return { viewPreferencesUpdate: { hasPersonal: true } };
  },
  ViewPrefsSetDefault: (v: Record<string, unknown>) => {
    log.push({ setDefault: v });
    return { viewPreferencesSetDefault: { hasPersonal: false, hasDefault: true } };
  },
});

describe("Projekt-Issues (docs/08 §2.7)", () => {
  it("zeigt Issues nach Meilenstein gruppiert, Seitenpanel mit Properties, Milestones und Progress", async () => {
    mockGraphql(handlers());
    await renderWithApp(
      <ProjectPage ws="acme" projectRef="P-ENG-1" tab="issues" />,
      newClient(),
      "/acme/project/P-ENG-1/issues",
    );
    expect(await screen.findByText("Erstes")).toBeInTheDocument();
    const group = await screen.findByRole("region", { name: "Beta" });
    expect(within(group).getByText("Erstes")).toBeInTheDocument();
    const panel = screen.getByRole("complementary", { name: "Project details" });
    expect(within(panel).getByRole("region", { name: "Properties" })).toBeInTheDocument();
    expect(within(panel).getByText(/50% of 2/)).toBeInTheDocument();
    expect(within(panel).getByText("No milestone")).toBeInTheDocument();
    const progress = within(panel).getByRole("region", { name: "Progress" });
    expect(within(progress).getByText("Scope")).toBeInTheDocument();
    expect(within(progress).getAllByText("3").length).toBeGreaterThan(0);
  });

  it("Seitenpanel ein/aus und Abschnitte einklappen", async () => {
    mockGraphql(handlers());
    await renderWithApp(
      <ProjectPage ws="acme" projectRef="P-ENG-1" tab="issues" />,
      newClient(),
      "/acme/project/P-ENG-1/issues",
    );
    const user = userEvent.setup();
    await screen.findByText("Erstes");
    await user.click(screen.getByRole("button", { name: "Collapse Milestones" }));
    const panel = screen.getByRole("complementary", { name: "Project details" });
    expect(within(panel).queryByText("No milestone")).toBeNull();
    await user.click(screen.getByRole("button", { name: "Hide side panel" }));
    expect(screen.queryByRole("complementary", { name: "Project details" })).toBeNull();
    expect(localStorage.getItem("lineal.project.panel")).toBe("0");
    await user.click(screen.getByRole("button", { name: "Show side panel" }));
    expect(screen.getByRole("complementary", { name: "Project details" })).toBeInTheDocument();
  });

  it("Anzeigeoptionen: Gruppierung wird je Projekt gespeichert, Standard für alle", async () => {
    const log: Array<Record<string, unknown>> = [];
    mockGraphql(handlers(log));
    await renderWithApp(
      <ProjectPage ws="acme" projectRef="P-ENG-1" tab="issues" />,
      newClient(),
      "/acme/project/P-ENG-1/issues",
    );
    const user = userEvent.setup();
    await screen.findByText("Erstes");
    await user.click(screen.getByRole("button", { name: "Display options" }));
    const dialog = await screen.findByRole("dialog", { name: "Display options" });
    await user.selectOptions(within(dialog).getByLabelText("Grouping"), "priority");
    await waitFor(() =>
      expect(log.at(-1)).toMatchObject({
        viewType: "projectIssues",
        projectId: "p-1",
        preferences: { grouping: "priority" },
      }),
    );
    await user.click(within(dialog).getByRole("button", { name: "Set default for everyone" }));
    await waitFor(() =>
      expect(log.at(-1)).toMatchObject({
        setDefault: { viewType: "projectIssues", projectId: "p-1" },
      }),
    );
  });

  it("Filtermenü: Kategorie wählen setzt einen Filterchip und lädt neu", async () => {
    mockGraphql(handlers());
    await renderWithApp(
      <ProjectPage ws="acme" projectRef="P-ENG-1" tab="issues" />,
      newClient(),
      "/acme/project/P-ENG-1/issues",
    );
    const user = userEvent.setup();
    await screen.findByText("Erstes");
    await user.click(screen.getByRole("button", { name: "Add filter" }));
    await user.click(await screen.findByRole("menuitem", { name: "Priority" }));
    await user.click(await screen.findByRole("checkbox", { name: "Urgent" }));
    await user.click(screen.getByRole("button", { name: "Apply" }));
    expect(await screen.findByRole("button", { name: "Edit filter Priority" })).toBeInTheDocument();
  });
});
