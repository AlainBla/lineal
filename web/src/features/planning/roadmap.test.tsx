import { fireEvent, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { mockGraphql, newClient, renderWithApp } from "@/features/issues/test-fixtures";
import type { Project } from "@/gql/graphql";

import { projectItems } from "./roadmap-items";
import {
  buildRuler,
  dragSpan,
  formatSpanDate,
  groupItems,
  normalizeDate,
  type RoadmapItem,
  type Span,
} from "./roadmap-model";
import { RoadmapPage, spanInput } from "./roadmap-page";

const ref = (id: string, name: string, key: string | null = null) => ({ id, name, key });

function project(over: Partial<Project> = {}): Project {
  return {
    id: "p-1",
    identifier: "P-ENG-1",
    previousIdentifiers: [],
    slugId: "abc",
    name: "Checkout Relaunch",
    summary: null,
    description: null,
    icon: null,
    color: "#4cb782",
    priority: 2,
    status: { id: "ps-1", name: "In Progress", type: "started", color: "#f2c94c" },
    health: null,
    progress: 0.4,
    scope: 10,
    completedScope: 4,
    issueCount: 10,
    completedIssueCount: 4,
    startDate: "2026-10-01",
    startDateResolution: null,
    targetDate: "2026-12-31",
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
    initiatives: [ref("i-1", "Growth")],
    milestones: [
      {
        id: "m-1",
        project: { id: "p-1", identifier: "P-ENG-1", name: "Checkout Relaunch" },
        name: "Beta",
        description: null,
        targetDate: "2026-11-15",
        sortOrder: 1,
        status: "unstarted",
        progress: 0,
        issueCount: 0,
        completedIssueCount: 0,
        createdAt: "2026-10-01T00:00:00Z",
        updatedAt: "2026-10-01T00:00:00Z",
      },
    ],
    openIssueCount: null,
    createdAt: "2026-10-01T00:00:00Z",
    updatedAt: "2026-10-01T00:00:00Z",
    ...over,
  };
}

const span = (over: Partial<Span> = {}): Span => ({
  startDate: "2026-10-01",
  targetDate: "2026-12-31",
  startRes: null,
  targetRes: null,
  ...over,
});

afterEach(() => vi.unstubAllGlobals());

describe("normalizeDate (docs/04 §2.5)", () => {
  it("start = Periodenbeginn, Ziel = Periodenende", () => {
    expect(normalizeDate("2026-11-14", "month", "start")).toBe("2026-11-01");
    expect(normalizeDate("2026-11-14", "month", "target")).toBe("2026-11-30");
    expect(normalizeDate("2026-11-14", "quarter", "start")).toBe("2026-10-01");
    expect(normalizeDate("2026-11-14", "quarter", "target")).toBe("2026-12-31");
    expect(normalizeDate("2026-11-14", "halfYear", "start")).toBe("2026-07-01");
    expect(normalizeDate("2026-11-14", "halfYear", "target")).toBe("2026-12-31");
    expect(normalizeDate("2026-02-10", "halfYear", "target")).toBe("2026-06-30");
    expect(normalizeDate("2026-11-14", "year", "start")).toBe("2026-01-01");
    expect(normalizeDate("2026-11-14", null, "target")).toBe("2026-11-14");
  });

  it("formats with resolution", () => {
    expect(formatSpanDate("2026-11-30", "month")).toBe("Nov 2026");
    expect(formatSpanDate("2026-12-31", "quarter")).toBe("Q4 2026");
    expect(formatSpanDate("2026-12-31", "halfYear")).toBe("H2 2026");
    expect(formatSpanDate("2026-12-31", "year")).toBe("2026");
    expect(formatSpanDate("2026-11-14", null)).toBe("Nov 14, 2026");
    expect(formatSpanDate(null, null)).toBe("–");
  });
});

describe("dragSpan", () => {
  it("verschiebt Start und Ziel um Tage", () => {
    expect(dragSpan(span(), "move", 10)).toMatchObject({
      startDate: "2026-10-11",
      targetDate: "2027-01-10",
    });
    expect(dragSpan(span(), "move", -1)).toMatchObject({
      startDate: "2026-09-30",
      targetDate: "2026-12-30",
    });
  });

  it("ändert mit start/end nur ein Ende; Start nach Ziel ist ungültig", () => {
    expect(dragSpan(span(), "end", 5)).toMatchObject({
      startDate: "2026-10-01",
      targetDate: "2027-01-05",
    });
    expect(dragSpan(span(), "start", 5)).toMatchObject({
      startDate: "2026-10-06",
      targetDate: "2026-12-31",
    });
    expect(dragSpan(span(), "start", 400)).toBeNull();
    expect(dragSpan(span(), "end", -400)).toBeNull();
    expect(dragSpan(span(), "move", 0)).toBeNull();
  });

  it("wandert bei Auflösung in ganzen Perioden und bleibt normalisiert", () => {
    const q = span({
      startDate: "2026-10-01",
      targetDate: "2026-12-31",
      startRes: "quarter",
      targetRes: "quarter",
    });
    expect(dragSpan(q, "move", 92)).toMatchObject({
      startDate: "2027-01-01",
      targetDate: "2027-03-31",
    });
    expect(dragSpan(q, "move", -92)).toMatchObject({
      startDate: "2026-07-01",
      targetDate: "2026-09-30",
    });
    // Weniger als eine halbe Periode: nichts.
    expect(dragSpan(q, "move", 20)).toBeNull();
    const m = span({
      startDate: "2026-01-01",
      targetDate: "2026-01-31",
      startRes: "month",
      targetRes: "month",
    });
    expect(dragSpan(m, "move", 31)).toMatchObject({
      startDate: "2026-02-01",
      targetDate: "2026-02-28",
    });
  });

  it("nur Zieldatum vorhanden", () => {
    expect(dragSpan(span({ startDate: null }), "move", 3)).toMatchObject({
      startDate: null,
      targetDate: "2027-01-03",
    });
  });
});

describe("Zeitachse und Gruppen", () => {
  const items = projectItems([project()], "acme");

  it("umfasst Elemente, Meilensteine und heute", () => {
    const r = buildRuler(items, "month", "2026-10-12");
    const end = r.origin + r.days;
    expect(r.origin).toBeLessThan(Date.UTC(2026, 9, 1) / 86_400_000);
    expect(end).toBeGreaterThan(Date.UTC(2026, 11, 31) / 86_400_000);
    expect(r.ticks[0]!.label).toMatch(/\d{4}$/);
    const w = buildRuler(items, "week", "2026-10-12");
    // Wochen beginnen am Montag.
    expect(new Date(w.ticks[0]!.day * 86_400_000).getUTCDay()).toBe(1);
    expect(buildRuler(items, "quarter", "2026-10-12").ticks.map((t) => t.label)).toContain(
      "Q4 2026",
    );
  });

  it("gruppiert nach Initiative, Team, Status", () => {
    const two: RoadmapItem[] = [
      ...items,
      ...projectItems(
        [project({ id: "p-2", name: "B", initiatives: [], leadTeam: null, teams: [] })],
        "acme",
      ),
    ];
    expect(groupItems(two, "initiative").map((g) => g.label)).toEqual(["Growth", "No initiative"]);
    expect(groupItems(two, "team").map((g) => g.label)).toEqual(["Engineering", "No team"]);
    expect(groupItems(two, "status")).toHaveLength(1);
    expect(groupItems(two, "none")).toHaveLength(1);
  });

  it("spanInput sendet nur Geändertes samt Auflösung", () => {
    const [it] = items;
    expect(spanInput(it!, span({ targetDate: "2027-01-05" }))).toEqual({
      targetDate: "2027-01-05",
    });
    const q = {
      ...it!,
      span: span({ startRes: "quarter", targetRes: "quarter" }),
    };
    expect(
      spanInput(
        q,
        span({
          startDate: "2027-01-01",
          targetDate: "2027-03-31",
          startRes: "quarter",
          targetRes: "quarter",
        }),
      ),
    ).toEqual({
      startDate: "2027-01-01",
      startDateResolution: "quarter",
      targetDate: "2027-03-31",
      targetDateResolution: "quarter",
    });
    // Initiativen: Start nicht editierbar.
    expect(spanInput({ ...it!, startEditable: false }, span({ startDate: "2026-10-09" }))).toEqual(
      {},
    );
  });
});

describe("RoadmapPage", () => {
  const setup = (calls: Array<Record<string, unknown>> = [], fail = false) =>
    mockGraphql({
      PlanningProjects: () => ({
        projects: [
          project(),
          project({
            id: "p-2",
            identifier: "P-ENG-2",
            name: "No dates",
            startDate: null,
            targetDate: null,
          }),
        ],
      }),
      PlanningInitiatives: () => ({ initiatives: [] }),
      PlanningProjectUpdate: (v: Record<string, unknown>) => {
        calls.push(v);
        if (fail) throw new Error("forbidden");
        return { projectUpdate: { success: true } };
      },
    });

  it("zeigt Balken, Meilenstein, heute-Linie und Elemente ohne Termine", async () => {
    setup();
    const { container } = await renderWithApp(
      <RoadmapPage ws="acme" scope={{ kind: "workspace" }} />,
      newClient(),
      "/acme/roadmap",
    );
    expect(
      await screen.findByRole("button", { name: /Checkout Relaunch, Oct 1, 2026 – Dec 31, 2026/ }),
    ).toBeInTheDocument();
    expect(container.querySelector("[data-milestone='m-1']")).not.toBeNull();
    expect(container.querySelector("[data-today]")).not.toBeNull();
    expect(screen.getByRole("link", { name: "No dates" })).toBeInTheDocument();
    for (const z of ["Week", "Month", "Quarter", "Year"]) {
      expect(screen.getByRole("radio", { name: z })).toBeInTheDocument();
    }
    await fireEvent.click(screen.getByRole("radio", { name: "Quarter" }));
    expect(screen.getByRole("radio", { name: "Quarter" })).toHaveAttribute("aria-checked", "true");
  });

  it("verschiebt per Pfeiltaste: projectUpdate mit neuen Daten", async () => {
    const calls: Array<Record<string, unknown>> = [];
    setup(calls);
    await renderWithApp(
      <RoadmapPage ws="acme" scope={{ kind: "workspace" }} />,
      newClient(),
      "/acme/roadmap",
    );
    const bar = await screen.findByRole("button", { name: /Checkout Relaunch/ });
    fireEvent.keyDown(bar, { key: "ArrowRight" });
    await waitFor(() => expect(calls).toHaveLength(1));
    expect(calls[0]).toEqual({
      id: "p-1",
      input: { startDate: "2026-10-02", targetDate: "2027-01-01" },
    });
    // Shift: nur das Zieldatum.
    fireEvent.keyDown(bar, { key: "ArrowLeft", shiftKey: true });
    await waitFor(() => expect(calls).toHaveLength(2));
    expect(calls[1]).toEqual({ id: "p-1", input: { targetDate: "2026-12-30" } });
  });

  it("zeigt den Serverfehler, wenn die Änderung abgelehnt wird", async () => {
    setup([], true);
    await renderWithApp(
      <RoadmapPage ws="acme" scope={{ kind: "workspace" }} />,
      newClient(),
      "/acme/roadmap",
    );
    const bar = await screen.findByRole("button", { name: /Checkout Relaunch/ });
    fireEvent.keyDown(bar, { key: "ArrowRight" });
    expect(await screen.findByRole("alert")).toBeInTheDocument();
  });
});
