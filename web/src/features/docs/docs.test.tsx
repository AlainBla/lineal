import { screen, waitFor, within } from "@testing-library/react";
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

  it("shows a title changed by someone else after the document is refetched", async () => {
    let title = "Launch plan";
    mockGraphql({ DocumentDetail: () => ({ document: { ...doc, title } }) });
    const client = newClient();
    await renderWithApp(<DocumentPage docRef="abc" />, client, "/acme/document/abc");
    const input = await screen.findByLabelText("Document title");
    expect(input).toHaveValue("Launch plan");
    title = "Renamed by Bob";
    await client.invalidateQueries();
    await waitFor(() => expect(input).toHaveValue("Renamed by Bob"));
  });

  it("subscribes via the bell and moves the document to another parent", async () => {
    let subscribed = false;
    const gql = mockGraphql({
      DocumentDetail: () => ({ document: doc }),
      DocumentSubscribed: () => ({ documentSubscribed: subscribed }),
      DocumentSubscribe: () => {
        subscribed = true;
        return { documentSubscribe: { success: true, subscribed: true } };
      },
      DocumentMove: () => ({
        documentUpdate: { success: true, document: { ...doc, projectId: "p-2" } },
      }),
    });
    await renderWithApp(<DocumentPage docRef="abc" />, newClient(), "/acme/document/abc");
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Subscribe to this document" }));
    expect(
      await screen.findByRole("button", { name: "Unsubscribe from this document" }),
    ).toHaveAttribute("aria-pressed", "true");
    expect(gql.of("DocumentSubscribe")[0]!.variables).toEqual({ id: "d1" });

    await user.click(screen.getByRole("button", { name: /Settings/ }));
    await user.type(screen.getByLabelText("New parent"), "Checkout");
    await user.click(screen.getByRole("button", { name: "Move" }));
    await waitFor(() =>
      expect(gql.of("DocumentMove")[0]!.variables).toEqual({
        id: "d1",
        input: { projectId: "Checkout" },
      }),
    );
  });

  it("shows the trashed banner with a restore action", async () => {
    const gql = mockGraphql({
      DocumentDetail: () => ({ document: { ...doc, trashed: true } }),
      DocumentSubscribed: () => ({ documentSubscribed: false }),
      DocumentRestoreTrash: () => ({ documentUnarchive: { success: true } }),
    });
    await renderWithApp(<DocumentPage docRef="abc" />, newClient(), "/acme/document/abc");
    const user = userEvent.setup();
    expect(await screen.findByText(/in Recently deleted/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /Restore$/ }));
    await waitFor(() => expect(gql.of("DocumentRestoreTrash")).toHaveLength(1));
  });

  it("lists inline threads in the sidebar and resolves one", async () => {
    const gql = mockGraphql({
      DocumentDetail: () => ({ document: doc }),
      DocumentSubscribed: () => ({ documentSubscribed: false }),
      TargetComments: () => ({
        comments: {
          nodes: [
            {
              id: "c1",
              body: "Please rephrase",
              userId: "user-alice",
              user: { id: "user-alice", name: "Alice", displayName: "Alice" },
              documentId: "d1",
              parentId: null,
              quotedText: "Hello",
              anchorStart: null,
              anchorEnd: null,
              anchorState: "attached",
              editedAt: null,
              resolvedAt: null,
              createdAt: "2026-10-01T10:00:00Z",
              updatedAt: "2026-10-01T10:00:00Z",
            },
          ],
          pageInfo: { hasNextPage: false, endCursor: null },
        },
      }),
      TargetCommentResolve: () => ({ commentResolve: { success: true } }),
    });
    await renderWithApp(<DocumentPage docRef="abc" />, newClient(), "/acme/document/abc");
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: /Comments \(1\)/ }));
    const panel = await screen.findByRole("complementary", { name: "Comment threads" });
    expect(within(panel).getByText("Please rephrase")).toBeInTheDocument();
    expect(within(panel).getByText("Hello")).toBeInTheDocument();
    await user.click(within(panel).getByRole("button", { name: "Resolve" }));
    await waitFor(() => expect(gql.of("TargetCommentResolve")[0]!.variables).toEqual({ id: "c1" }));
  });
});
