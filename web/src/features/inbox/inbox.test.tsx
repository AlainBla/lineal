import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { Notification } from "@/gql/graphql";
import { makeIssue, mockGraphql, newClient, renderWithApp } from "@/features/issues/test-fixtures";
import { useRouterState } from "@tanstack/react-router";

import { InboxPage } from "./inbox-page";
import { applyInboxAction, snoozePresets } from "./queries";
import { attachShortcuts, shortcuts } from "@/lib/shortcuts";

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
    projectMilestoneId: null,
    projectUpdateId: null,
    initiativeUpdateId: null,
    bundledTypes: [],
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

  it("shows type badges for new types and a bundle counter with the merged changes", async () => {
    mockGraphql({
      InboxNotifications: () => ({
        notifications: {
          nodes: [
            note("n1", { type: "issueUnassignedFromYou", title: "Login kaputt" }),
            note("n2", {
              type: "issueAssignedToYou",
              title: "Checkout hängt",
              bundledTypes: ["issueAssignedToYou", "issueNewComment", "issueStatusChanged"],
            }),
            note("n3", { type: "documentNewComment", title: "Spezifikation", documentId: "d1" }),
          ],
          pageInfo: { hasNextPage: false, endCursor: null },
        },
      }),
    });
    await renderWithApp(<InboxPage ws="acme" />, newClient(), "/acme/inbox");
    await screen.findByText("Login kaputt");
    const badges = screen.getAllByTestId("type-badge").map((b) => b.textContent);
    expect(badges).toEqual(["Unassigned", "Assigned to you", "Document comment"]);
    // Nur die gebündelte Notification trägt den Zähler; Tooltip nennt alle Änderungen.
    const counter = screen.getAllByTestId("bundle-badge");
    expect(counter).toHaveLength(1);
    expect(counter[0]).toHaveTextContent("3");
    expect(counter[0]).toHaveAttribute("title", "Assigned to you, Comment, Status changed");
    // Im Detail (nicht-Issue-Eintrag) erscheint die Art des Bezugsobjekts.
    const user = userEvent.setup();
    await user.click(screen.getByText("Spezifikation"));
    expect(await screen.findByText("document")).toBeInTheDocument();
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

  it("Issue-Eintrag bleibt in der Split-Ansicht, URL bleibt /inbox (B4.01)", async () => {
    mockGraphql({
      InboxNotifications: () => ({
        notifications: {
          nodes: [note("n1", { issueIdentifier: "ENG-120", issueId: "i1" }), note("n2")],
          pageInfo: { hasNextPage: false, endCursor: null },
        },
      }),
      InboxMarkRead: () => ({ notificationUpdate: { success: true } }),
      IssueDetail: () => ({
        issue: makeIssue({ id: "i1", identifier: "ENG-120", title: "Checkout überarbeiten" }),
      }),
    });
    function Path() {
      const p = useRouterState({ select: (st) => st.location.pathname });
      return <span data-testid="path">{p}</span>;
    }
    await renderWithApp(
      <>
        <Path />
        <InboxPage ws="acme" />
      </>,
      newClient(),
      "/acme/inbox",
    );
    const user = userEvent.setup();
    await user.click(await screen.findByText("Note n1"));
    expect(await screen.findByLabelText("Title")).toHaveValue("Checkout überarbeiten");
    // Auch nach dem Laden darf die eingebettete Issue-Seite die URL nicht auf /issue/… umschreiben.
    await new Promise((r) => setTimeout(r, 50));
    expect(screen.getByTestId("path")).toHaveTextContent("/acme/inbox");
    expect(screen.getByText("Note n2")).toBeInTheDocument();
    await user.keyboard("j");
    expect(screen.getByTestId("path")).toHaveTextContent("/acme/inbox");
  });

  it("Taste H öffnet das Snooze-Menü mit This evening und Custom (B4.38)", async () => {
    const gql = mockGraphql({
      InboxNotifications: () => ({
        notifications: { nodes: [note("n1")], pageInfo: { hasNextPage: false, endCursor: null } },
      }),
      InboxMarkRead: () => ({ notificationUpdate: { success: true } }),
      InboxSnooze: () => ({ notificationUpdate: { success: true } }),
    });
    const detach = attachShortcuts();
    await renderWithApp(<InboxPage ws="acme" />, newClient(), "/acme/inbox");
    const user = userEvent.setup();
    await user.click(await screen.findByText("Note n1"));
    await user.keyboard("h");
    expect(await screen.findByRole("menuitem", { name: "Custom…" })).toBeInTheDocument();
    await user.click(screen.getByRole("menuitem", { name: "This evening" }));
    await waitFor(() => expect(gql.of("InboxSnooze")).toHaveLength(1));
    detach();
    shortcuts.reset();
  });

  it("snoozePresets bietet Next cycle nur mit künftigem Cycle-Start (B4.38)", () => {
    const now = new Date("2026-10-13T08:00:00Z");
    expect(snoozePresets(now).some((p) => p.id === "cycle")).toBe(false);
    expect(snoozePresets(now, new Date("2026-10-12T00:00:00Z")).some((p) => p.id === "cycle")).toBe(
      false,
    );
    const start = new Date("2026-10-19T00:00:00Z");
    const cycle = snoozePresets(now, start).find((p) => p.id === "cycle");
    expect(cycle?.label).toBe("Next cycle");
    expect(cycle?.until).toEqual(start);
  });

  it("snoozePresets enthält This evening (18 Uhr, danach morgen)", () => {
    const morning = new Date(2026, 9, 13, 10, 0);
    const ev = snoozePresets(morning).find((p) => p.id === "evening")!;
    expect(ev.until.getHours()).toBe(18);
    expect(ev.until.getDate()).toBe(13);
    const late = new Date(2026, 9, 13, 19, 0);
    expect(
      snoozePresets(late)
        .find((p) => p.id === "evening")!
        .until.getDate(),
    ).toBe(14);
  });
});
