import { fireEvent, screen, waitFor, within } from "@testing-library/react";
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
import { attachShortcuts, shortcuts } from "@/lib/shortcuts";

import { MilestonePage } from "./milestone-page";

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

const MILESTONE = {
  id: "m-1",
  name: "Go-Live",
  description: "Rollback ist der DNS-Wechsel.",
  targetDate: "2026-12-18",
  sortOrder: 1,
  status: "next",
  progress: 0.5,
  issueCount: 4,
  completedIssueCount: 2,
  startedIssueCount: 1,
  readyToComplete: false,
  completedAt: null,
  resultNote: null,
  hasSnapshot: false,
  addedIssueIds: ["b"],
  targetDateHistory: [
    { fromDate: "2026-12-04", toDate: "2026-12-18", changedAt: "2026-11-02T10:00:00Z" },
  ],
};

const PROJECT = {
  id: "p-1",
  identifier: "P-ENG-1",
  slugId: "abc",
  name: "Relaunch",
  summary: null,
  description: null,
  status: { id: "s", name: "Started", type: "started", color: "#000" },
  leadTeam: ref("team-eng", "Engineering", "ENG"),
  teams: [ref("team-eng", "Engineering", "ENG")],
  members: [],
  labels: [],
  initiatives: [],
  milestones: [MILESTONE],
};

const ISSUES = [
  makeIssue({
    id: "a",
    identifier: "ENG-1",
    title: "Redirects",
    stateName: "In Progress",
    stateType: "started",
    projectId: "p-1",
    projectMilestoneId: "m-1",
  }),
  makeIssue({
    id: "b",
    identifier: "ENG-2",
    title: "DNS vorbereiten",
    stateName: "Todo",
    stateType: "unstarted",
    projectId: "p-1",
    projectMilestoneId: "m-1",
  }),
  makeIssue({
    id: "c",
    identifier: "ENG-3",
    title: "Inhalte migriert",
    stateName: "Done",
    stateType: "completed",
    projectId: "p-1",
    projectMilestoneId: "m-1",
  }),
];

const RESOURCES = {
  documents: [{ id: "d-1", title: "Checkliste", slugId: "abc123abc123" }],
  links: [{ id: "l-1", url: "https://example.com/r", label: "Bericht" }],
  files: [
    {
      id: "f-1",
      title: "plan.pdf",
      contentType: "application/pdf",
      size: 2048,
      downloadUrl: "/files/signed",
      createdAt: "2026-01-01T00:00:00Z",
    },
  ],
};
const OTHER_DOC = { id: "d-2", title: "Konzept", slugId: "def456def456" };

const handlers = (over: Record<string, unknown> = {}) => ({
  PlanningProject: () => ({ project: PROJECT }),
  ViewIssuesList: () => ({ viewIssues: { nodes: ISSUES, hasNextPage: false, endCursor: null } }),
  TargetComments: () => ({
    comments: { nodes: [], pageInfo: { hasNextPage: false, endCursor: null } },
  }),
  TeamMeta: () => ({
    team: TEAM_META.team,
    workflowStates: TEAM_META.states,
    issueLabels: TEAM_META.labels,
    cycles: [],
    projects: TEAM_META.projects,
  }),
  IssueTeams: () => ({ teams: [] }),
  PlanningResources: (v: Record<string, unknown>) => ({
    resources: v.milestoneId ? RESOURCES : { documents: [OTHER_DOC], links: [], files: [] },
  }),
  ...over,
});

const open = (id = "m-1") =>
  renderWithApp(
    <MilestonePage ws="acme" projectRef="P-ENG-1" milestoneId={id} />,
    newClient(),
    `/acme/project/P-ENG-1/milestone/${id}`,
  );

let detach: () => void;
beforeEach(() => {
  localStorage.clear();
  detach = attachShortcuts();
});
afterEach(() => {
  detach();
  vi.unstubAllGlobals();
  shortcuts.reset();
});

describe("Meilenstein-Detailseite (LIN-25)", () => {
  it("zeigt Kopf, Zähler statt Prozent und die Beschreibung", async () => {
    mockGraphql(handlers());
    await open();
    expect(await screen.findByRole("heading", { name: "Go-Live" })).toBeInTheDocument();
    expect(screen.getByText("Next")).toBeInTheDocument();
    const summary = screen.getByRole("group", { name: "Progress of Go-Live" });
    expect(within(summary).getByText("2 of 4 done")).toBeInTheDocument();
    expect(within(summary).getByText("1 in progress")).toBeInTheDocument();
    expect(within(summary).getByText("1 not started")).toBeInTheDocument();
    expect(await screen.findByText(/Rollback ist der DNS-Wechsel/)).toBeInTheDocument();
  });

  it("gruppiert Issues nach Status und klappt Erledigte ein", async () => {
    mockGraphql(handlers());
    const user = userEvent.setup();
    await open();
    const section = await screen.findByRole("region", { name: "Issues" });
    expect(await within(section).findByText("Redirects")).toBeInTheDocument();
    expect(within(section).getByText("DNS vorbereiten")).toBeInTheDocument();
    expect(within(section).queryByText("Inhalte migriert")).toBeNull();
    const labels = within(section)
      .getAllByRole("button", { expanded: true })
      .concat(within(section).getAllByRole("button", { expanded: false }))
      .map((b) => b.textContent);
    expect(labels[0]).toContain("In Progress");
    expect(labels[1]).toContain("Todo");
    await user.click(within(section).getByRole("button", { name: /Done/ }));
    expect(within(section).getByText("Inhalte migriert")).toBeInTheDocument();
  });

  it("fragt die Issues nur dieses Meilensteins ab", async () => {
    const gql = mockGraphql(handlers());
    await open();
    await screen.findByText("Redirects");
    expect(gql.of("ViewIssuesList")[0]!.variables.filter).toEqual({
      projectMilestone: { id: { in: ["m-1"] } },
    });
  });

  it("speichert Name und Zieldatum auf der Seite", async () => {
    const gql = mockGraphql(
      handlers({ PlanningMilestoneUpdate: () => ({ projectMilestoneUpdate: { success: true } }) }),
    );
    const user = userEvent.setup();
    await open();
    const name = await screen.findByLabelText("Milestone name");
    await user.clear(name);
    await user.type(name, "Launch{Enter}");
    await waitFor(() => expect(gql.of("PlanningMilestoneUpdate")).toHaveLength(1));
    expect(gql.of("PlanningMilestoneUpdate")[0]!.variables).toMatchObject({
      projectId: "p-1",
      id: "m-1",
      input: { name: "Launch" },
    });
  });

  it("ändert das Zieldatum auf der Seite", async () => {
    const gql = mockGraphql(
      handlers({ PlanningMilestoneUpdate: () => ({ projectMilestoneUpdate: { success: true } }) }),
    );
    await open();
    fireEvent.change(await screen.findByLabelText("Target date"), {
      target: { value: "2026-12-24" },
    });
    await waitFor(() =>
      expect(gql.of("PlanningMilestoneUpdate").at(-1)!.variables).toMatchObject({
        input: { targetDate: "2026-12-24" },
      }),
    );
  });

  it("kommentiert am Meilenstein", async () => {
    const gql = mockGraphql(handlers());
    await open();
    await screen.findByRole("heading", { name: "Go-Live" });
    await waitFor(() => expect(gql.of("TargetComments")).toHaveLength(1));
    expect(gql.of("TargetComments")[0]!.variables).toMatchObject({ id: "m-1" });
    expect(screen.getByRole("region", { name: "Activity" })).toBeInTheDocument();
  });

  it("lädt die Zähler neu, wenn sich die Issue-Liste ändert", async () => {
    let nodes = ISSUES;
    const gql = mockGraphql(
      handlers({
        ViewIssuesList: () => ({ viewIssues: { nodes, hasNextPage: false, endCursor: null } }),
      }),
    );
    const client = newClient();
    await renderWithApp(
      <MilestonePage ws="acme" projectRef="P-ENG-1" milestoneId="m-1" />,
      client,
      "/acme/project/P-ENG-1/milestone/m-1",
    );
    await screen.findByText("Redirects");
    const before = gql.of("PlanningProject").length;
    nodes = ISSUES.slice(0, 2);
    await client.invalidateQueries({ queryKey: ["issues"] });
    await waitFor(() => expect(gql.of("PlanningProject").length).toBeGreaterThan(before));
  });

  it("zeigt Verschiebung, Terminverlauf und später hinzugekommene Issues", async () => {
    mockGraphql(handlers());
    await open();
    expect(await screen.findByText(/moved once \(\+14 days\)/)).toBeInTheDocument();
    const history = screen.getByRole("region", { name: "Target date history" });
    expect(within(history).getAllByRole("listitem")).toHaveLength(2);
    expect(screen.getByText("1 added since start")).toBeInTheDocument();
    const section = screen.getByRole("region", { name: "Issues" });
    const row = (await within(section).findByText("DNS vorbereiten")).closest("a")!;
    expect(within(row).getByText("added")).toBeInTheDocument();
    expect(
      within((await within(section).findByText("Redirects")).closest("a")!).queryByText("added"),
    ).toBeNull();
  });

  it("schließt bewusst ab und fragt nach offenen Issues", async () => {
    const other = { ...MILESTONE, id: "m-2", name: "Nachbereitung", addedIssueIds: [] };
    const gql = mockGraphql(
      handlers({
        PlanningProject: () => ({ project: { ...PROJECT, milestones: [MILESTONE, other] } }),
        PlanningMilestoneComplete: () => ({ projectMilestoneComplete: { success: true } }),
      }),
    );
    const user = userEvent.setup();
    await open();
    await user.click(await screen.findByRole("button", { name: "Complete milestone" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/2 open issues/)).toBeInTheDocument();
    expect(within(dialog).getByLabelText("Target milestone")).toHaveValue("m-2");
    await user.type(within(dialog).getByLabelText("Result note"), "Live.");
    await user.click(within(dialog).getByRole("button", { name: "Complete milestone" }));
    await waitFor(() => expect(gql.of("PlanningMilestoneComplete")).toHaveLength(1));
    expect(gql.of("PlanningMilestoneComplete")[0]!.variables).toMatchObject({
      id: "m-1",
      input: { openIssues: "move", moveTo: "m-2", note: "Live." },
    });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("zeigt am abgeschlossenen Meilenstein Ergebnis und eingefrorene Liste", async () => {
    const done = {
      ...MILESTONE,
      status: "done",
      completedAt: "2026-11-01T10:00:00Z",
      resultNote: "Live seit Freitag.",
      hasSnapshot: true,
    };
    const gql = mockGraphql(
      handlers({
        PlanningProject: () => ({ project: { ...PROJECT, milestones: [done] } }),
        PlanningMilestoneSnapshot: () => ({
          milestoneSnapshot: [
            {
              id: "a",
              identifier: "ENG-1",
              title: "Damals offen",
              stateName: "In Progress",
              stateType: "started",
            },
          ],
        }),
        PlanningMilestoneReopen: () => ({ projectMilestoneReopen: { success: true } }),
      }),
    );
    const user = userEvent.setup();
    await open();
    expect(await screen.findByText("Damals offen")).toBeInTheDocument();
    expect(await screen.findByText("Live seit Freitag.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Complete milestone" })).toBeNull();
    expect(gql.of("ViewIssuesList")).toHaveLength(0);
    await user.click(screen.getByRole("button", { name: "Reopen" }));
    await waitFor(() => expect(gql.of("PlanningMilestoneReopen")).toHaveLength(1));
  });

  it("wandelt nach Rückfrage in ein Projekt um", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValueOnce(false).mockReturnValue(true);
    const gql = mockGraphql(
      handlers({
        PlanningMilestoneConvert: () => ({
          projectMilestoneConvert: { project: { identifier: "P-ENG-9" } },
        }),
      }),
    );
    const user = userEvent.setup();
    await open();
    const run = async () => {
      await user.click(await screen.findByRole("button", { name: "Milestone actions Go-Live" }));
      await user.click(await screen.findByRole("menuitem", { name: /Convert to project/ }));
    };
    await run();
    expect(gql.of("PlanningMilestoneConvert")).toHaveLength(0);
    await run();
    await waitFor(() => expect(gql.of("PlanningMilestoneConvert")).toHaveLength(1));
    expect(confirm).toHaveBeenCalledTimes(2);
  });

  it("schlägt das Abschließen nur vor, wenn alles erledigt ist", async () => {
    mockGraphql(
      handlers({
        PlanningProject: () => ({
          project: {
            ...PROJECT,
            milestones: [{ ...MILESTONE, status: "next", readyToComplete: true }],
          },
        }),
      }),
    );
    await open();
    expect(await screen.findByText(/All issues are done/)).toBeInTheDocument();
    expect(screen.getByText("Next")).toBeInTheDocument();
  });

  it("zeigt Dokumente, Dateien und Links und entfernt sie", async () => {
    const gql = mockGraphql(
      handlers({
        PlanningResourceLinkRemove: () => ({ resourceLinkRemove: { success: true } }),
        PlanningResourceDocumentSet: () => ({ resourceDocumentSet: { success: true } }),
      }),
    );
    const user = userEvent.setup();
    await open();
    const section = await screen.findByRole("region", { name: "Resources" });
    expect(await within(section).findByRole("link", { name: "Checkliste" })).toHaveAttribute(
      "href",
      "/acme/document/abc123abc123",
    );
    expect(within(section).getByRole("link", { name: "plan.pdf" })).toHaveAttribute(
      "href",
      "/files/signed",
    );
    expect(within(section).getByRole("link", { name: "Bericht" })).toHaveAttribute(
      "href",
      "https://example.com/r",
    );
    await user.click(within(section).getByRole("button", { name: "Remove Bericht" }));
    await waitFor(() => expect(gql.of("PlanningResourceLinkRemove")).toHaveLength(1));
    await user.click(within(section).getByRole("button", { name: "Remove Checkliste" }));
    await waitFor(() => expect(gql.of("PlanningResourceDocumentSet")).toHaveLength(1));
    expect(gql.of("PlanningResourceDocumentSet")[0]!.variables).toMatchObject({
      documentId: "d-1",
    });
    expect(gql.of("PlanningResourceDocumentSet")[0]!.variables.milestoneId).toBeUndefined();
  });

  it("fügt einen Link und ein vorhandenes Dokument hinzu", async () => {
    const gql = mockGraphql(
      handlers({
        PlanningResourceLinkAdd: () => ({ resourceLinkAdd: { success: true } }),
        PlanningResourceDocumentSet: () => ({ resourceDocumentSet: { success: true } }),
      }),
    );
    const user = userEvent.setup();
    await open();
    await user.click(await screen.findByRole("button", { name: "Add resource" }));
    await user.click(await screen.findByRole("menuitem", { name: "Add link" }));
    await user.type(screen.getByLabelText("Link URL"), "https://example.com/x");
    await user.click(screen.getByRole("button", { name: "Add" }));
    await waitFor(() => expect(gql.of("PlanningResourceLinkAdd")).toHaveLength(1));
    expect(gql.of("PlanningResourceLinkAdd")[0]!.variables).toMatchObject({
      milestoneId: "m-1",
      url: "https://example.com/x",
      label: "https://example.com/x",
    });

    await user.click(screen.getByRole("button", { name: "Add resource" }));
    await user.click(await screen.findByRole("menuitem", { name: /Existing document/ }));
    await user.selectOptions(await screen.findByLabelText("Existing document"), "d-2");
    await waitFor(() => expect(gql.of("PlanningResourceDocumentSet")).toHaveLength(1));
    expect(gql.of("PlanningResourceDocumentSet")[0]!.variables).toMatchObject({
      documentId: "d-2",
      milestoneId: "m-1",
    });
  });

  it("zeigt den Serverfehler, wenn der Link abgelehnt wird", async () => {
    mockGraphql(
      handlers({ PlanningResourceLinkAdd: () => new Error("links must use http or https") }),
    );
    const user = userEvent.setup();
    await open();
    await user.click(await screen.findByRole("button", { name: "Add resource" }));
    await user.click(await screen.findByRole("menuitem", { name: "Add link" }));
    await user.type(screen.getByLabelText("Link URL"), "ftp://x");
    await user.click(screen.getByRole("button", { name: "Add" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("links must use http or https");
  });

  it("zeigt einen Hinweis, wenn der Meilenstein nicht zum Projekt gehört", async () => {
    mockGraphql(handlers());
    await open("m-weg");
    expect(await screen.findByText("Milestone not found")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Back to milestones" })).toHaveAttribute(
      "href",
      "/acme/project/P-ENG-1/milestones",
    );
  });

  it("zeigt den Serverfehler, wenn das Projekt nicht lädt", async () => {
    mockGraphql(handlers({ PlanningProject: () => new Error("Project not found") }));
    await open();
    expect(await screen.findByText(/Project not found/)).toBeInTheDocument();
  });
});
