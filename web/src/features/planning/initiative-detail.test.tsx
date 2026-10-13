import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { mockGraphql, newClient, renderWithApp } from "@/features/issues/test-fixtures";
import type { Initiative } from "@/gql/graphql";

import { InitiativeDetail, INITIATIVE_TABS, isInitiativeTab } from "./initiative-detail";

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

const ref = (id: string, name: string) => ({ id, name, key: null });

const initiative = (): Initiative => ({
  id: "i-1",
  identifier: "I-1",
  slugId: "abc",
  name: "Growth",
  summary: "Grow the base",
  description: null,
  color: null,
  icon: null,
  status: "Active",
  statusType: "active",
  priority: 2,
  health: "onTrack",
  targetDate: "2026-12-31",
  targetDateResolution: null,
  startedAt: null,
  completedAt: null,
  owner: ref("u-1", "Alice"),
  leadTeam: null,
  visibility: "public",
  parentInitiatives: [],
  subInitiatives: [ref("i-2", "Activation")],
  labels: [],
  projects: [
    {
      id: "p-1",
      identifier: "P-ENG-1",
      name: "Checkout",
      status: "In Progress",
      statusType: "started",
      health: "atRisk",
    },
  ],
  healthRollup: { color: "atRisk", onTrack: 0, atRisk: 1, offTrack: 0, none: 0 },
  archivedAt: null,
  createdAt: "2026-10-01T00:00:00Z",
  updatedAt: "2026-10-01T00:00:00Z",
});

afterEach(() => vi.unstubAllGlobals());

describe("InitiativeDetail", () => {
  const handlers = (posted: string[] = []) => ({
    PlanningInitiative: () => ({ initiative: initiative() }),
    PlanningInitiativeUpdates: () => ({ initiativeUpdates: [] }),
    PlanningInitiativeUpdateCreate: (v: Record<string, unknown>) => {
      posted.push(`${String(v.health)}:${String(v.body)}`);
      return { initiativeUpdateCreate: { success: true } };
    },
    DocumentList: () => ({
      documents: {
        nodes: [
          {
            id: "d-1",
            slugId: "doc1",
            title: "Strategy",
            icon: null,
            updatedAt: "2026-10-02T00:00:00Z",
          },
        ],
      },
    }),
  });
  const render = (tab: (typeof INITIATIVE_TABS)[number], posted?: string[]) => {
    mockGraphql(handlers(posted));
    return renderWithApp(
      <InitiativeDetail ws="acme" initiativeRef="I-1" tab={tab} />,
      newClient(),
      `/acme/initiative/I-1/${tab}`,
    );
  };

  it("kennt alle Tabs laut docs/04", () => {
    expect(INITIATIVE_TABS.filter((t) => t !== "roadmap")).toEqual([
      "overview",
      "projects",
      "updates",
      "documents",
      "sub-initiatives",
    ]);
    expect(isInitiativeTab("updates")).toBe(true);
    expect(isInitiativeTab("nope")).toBe(false);
  });

  it("Kopf mit Tab-Links und Übersicht", async () => {
    await render("overview");
    expect(await screen.findByRole("heading", { name: "Growth" })).toBeInTheDocument();
    for (const t of [
      "Overview",
      "Projects",
      "Updates",
      "Documents",
      "Sub-initiatives",
      "Roadmap",
    ]) {
      expect(screen.getByRole("link", { name: t })).toBeInTheDocument();
    }
    expect(screen.getByDisplayValue("Grow the base")).toBeInTheDocument();
    expect(screen.getByText(/1 at risk/)).toBeInTheDocument();
  });

  it("Projekte, Dokumente, Sub-Initiativen", async () => {
    await render("projects");
    expect(await screen.findByRole("link", { name: "Checkout" })).toBeInTheDocument();
  });

  it("Dokumente-Tab listet Dokumente der Initiative", async () => {
    await render("documents");
    expect(await screen.findByRole("link", { name: "Strategy" })).toHaveAttribute(
      "href",
      "/acme/document/doc1",
    );
  });

  it("Sub-Initiativen-Tab", async () => {
    await render("sub-initiatives");
    expect(await screen.findByRole("link", { name: "Activation" })).toBeInTheDocument();
  });

  it("Updates-Tab postet ein Update mit Health", async () => {
    const posted: string[] = [];
    await render("updates", posted);
    const user = userEvent.setup();
    await screen.findByRole("form", { name: "Post update" });
    await user.click(screen.getByRole("radio", { name: "At risk" }));
    await user.type(screen.getByLabelText("Update text"), "Behind plan");
    await user.click(screen.getByRole("button", { name: "Post update" }));
    await waitFor(() => expect(posted).toEqual(["atRisk:Behind plan"]));
  });
});
