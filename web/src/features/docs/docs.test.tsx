import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { mockGraphql, newClient, renderWithApp } from "@/features/issues/test-fixtures";
import type { Document } from "@/gql/graphql";

import { DocumentPage } from "./document-page";

// Stufe 1 (Collab-Endpunkt nicht erreichbar): die Echtzeit-Tests liegen in features/collab.
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

const doc: Document = {
  id: "d1",
  slugId: "abc",
  title: "Launch plan",
  content: "Hello",
  icon: null,
  color: null,
  url: "/acme/document/abc",
  sortOrder: 0,
  creatorId: null,
  updatedById: null,
  projectId: null,
  issueId: null,
  initiativeId: null,
  cycleId: null,
  teamId: null,
  contentVersion: 3,
  trashed: false,
  createdAt: "2026-10-01T00:00:00Z",
  updatedAt: "2026-10-01T00:00:00Z",
  archivedAt: null,
  warnings: [],
};

afterEach(() => vi.unstubAllGlobals());

describe("DocumentPage", () => {
  it("renders the title and lists history with restore", async () => {
    const gql = mockGraphql({
      DocumentDetail: () => ({ document: doc }),
      DocumentHistory: () => ({
        documentContentHistory: {
          nodes: [
            {
              id: "h1",
              markdown: "Old",
              title: null,
              actorIds: [],
              kind: "edit",
              restoredFromId: null,
              contentSnapshotAt: "2026-10-01T00:00:00Z",
            },
          ],
        },
      }),
      DocumentRestore: () => ({ documentContentRestore: { success: true } }),
    });
    await renderWithApp(<DocumentPage docRef="abc" />, newClient(), "/acme/document/abc");
    const user = userEvent.setup();
    expect(await screen.findByLabelText("Document title")).toHaveValue("Launch plan");
    await user.click(screen.getByRole("button", { name: /History/ }));
    await user.click(await screen.findByRole("button", { name: /Restore version/ }));
    await waitFor(() =>
      expect(gql.of("DocumentRestore")[0]!.variables).toEqual({
        documentId: "d1",
        historyId: "h1",
      }),
    );
  });

  it("autosaves the title with the expected version", async () => {
    const gql = mockGraphql({
      DocumentDetail: () => ({ document: doc }),
      DocumentSave: () => ({
        documentUpdate: { success: true, document: { ...doc, contentVersion: 4 } },
      }),
    });
    await renderWithApp(<DocumentPage docRef="abc" />, newClient(), "/acme/document/abc");
    const user = userEvent.setup();
    const input = await screen.findByLabelText("Document title");
    await user.type(input, "!");
    await waitFor(() => expect(gql.of("DocumentSave")).toHaveLength(1), { timeout: 3000 });
    expect(gql.of("DocumentSave")[0]!.variables).toEqual({
      id: "d1",
      input: { title: "Launch plan!", expectedVersion: 3 },
    });
  });
});
