import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { Notification } from "@/gql/graphql";
import { mockGraphql, newClient, renderWithApp } from "@/features/issues/test-fixtures";

import { InboxPage } from "./inbox-page";
import { applyInboxAction } from "./queries";

function note(id: string, over: Partial<Notification> = {}): Notification {
  return {
    id,
    type: "comment",
    category: "comments",
    title: `Note ${id}`,
    subtitle: "Alice commented",
    url: "/acme/inbox",
    groupKey: id,
    actorId: null,
    actorName: "Alice",
    issueId: null,
    issueIdentifier: null,
    projectId: null,
    initiativeId: null,
    documentId: null,
    commentId: null,
    isPriority: false,
    readAt: null,
    snoozedUntilAt: null,
    unsnoozedAt: null,
    archivedAt: null,
    createdAt: "2026-10-01T10:00:00Z",
    updatedAt: "2026-10-01T10:00:00Z",
    ...over,
  };
}

afterEach(() => vi.unstubAllGlobals());

describe("applyInboxAction", () => {
  const list = [note("a"), note("b", { readAt: "2026-10-02T00:00:00Z" })];
  it("toggles read", () => {
    expect(applyInboxAction(list, { kind: "read", id: "a", read: true }, "NOW")[0]!.readAt).toBe(
      "NOW",
    );
    expect(applyInboxAction(list, { kind: "read", id: "b", read: false }, "NOW")[1]!.readAt).toBe(
      null,
    );
  });
  it("removes archived and snoozed", () => {
    expect(applyInboxAction(list, { kind: "archive", id: "a" }, "N")).toHaveLength(1);
    expect(applyInboxAction(list, { kind: "snooze", id: "a", until: "x" }, "N")).toHaveLength(1);
  });
  it("archive all read keeps unread", () => {
    const out = applyInboxAction(list, { kind: "archiveAll", filter: "read" }, "N");
    expect(out.map((n) => n.id)).toEqual(["a"]);
  });
});

describe("InboxPage", () => {
  it("opens, marks read, archives and refreshes the sidebar badge", async () => {
    const archived = new Set<string>();
    const gql = mockGraphql({
      InboxNotifications: () => ({
        notifications: {
          nodes: [note("n1"), note("n2")].filter((n) => !archived.has(n.id)),
          pageInfo: { hasNextPage: false, endCursor: null },
        },
      }),
      InboxMarkRead: () => ({ notificationUpdate: { success: true } }),
      InboxArchive: (v) => {
        archived.add(String(v.id));
        return { notificationArchive: { success: true } };
      },
    });
    const client = newClient();
    const spy = vi.spyOn(client, "invalidateQueries");
    await renderWithApp(<InboxPage ws="acme" />, client, "/acme/inbox");
    const user = userEvent.setup();
    await screen.findByText("Note n1");
    expect(screen.getByText("2 unread")).toBeInTheDocument();

    await user.click(screen.getByText("Note n1"));
    await waitFor(() => expect(gql.of("InboxMarkRead")).toHaveLength(1));
    expect(gql.of("InboxMarkRead")[0]!.variables).toEqual({ id: "n1", input: { read: true } });

    await user.click(screen.getByRole("button", { name: "Delete notification" }));
    await waitFor(() => expect(gql.of("InboxArchive")).toHaveLength(1));
    expect(gql.of("InboxArchive")[0]!.variables).toEqual({ id: "n1" });
    await waitFor(() => expect(screen.queryByText("Note n1")).not.toBeInTheDocument());
    expect(spy).toHaveBeenCalledWith({ queryKey: ["sidebar-data"] });
  });

  it("mark all as read sends the batch mutation", async () => {
    const gql = mockGraphql({
      InboxNotifications: () => ({
        notifications: { nodes: [note("n1")], pageInfo: { hasNextPage: false, endCursor: null } },
      }),
      InboxMarkAllRead: () => ({ notificationMarkReadAll: { success: true, count: 1 } }),
    });
    await renderWithApp(<InboxPage ws="acme" />, newClient(), "/acme/inbox");
    const user = userEvent.setup();
    await screen.findByText("Note n1");
    await user.click(screen.getByRole("button", { name: /Mark all as read/ }));
    await waitFor(() => expect(gql.of("InboxMarkAllRead")).toHaveLength(1));
  });
});
