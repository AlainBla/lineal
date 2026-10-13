import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Notification } from "@/gql/graphql";
import { mockGraphql, newClient, renderWithApp } from "@/features/issues/test-fixtures";

import { favoriteTarget } from "./inbox-actions";
import { InboxPage } from "./inbox-page";
import {
  applyFilters,
  buildInbox,
  DEFAULT_PREFS,
  groupNotifications,
  NO_FILTERS,
  setInboxPrefs,
} from "./inbox-model";

function note(id: string, over: Partial<Notification> = {}): Notification {
  return {
    id,
    type: "issueNewComment",
    category: "commentsAndReplies",
    title: `Note ${id}`,
    subtitle: "Alice commented",
    url: "/acme/inbox",
    groupKey: id,
    actorId: "u-alice",
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

const page = (nodes: Notification[]) => ({
  notifications: { nodes, pageInfo: { hasNextPage: false, endCursor: null } },
});

beforeEach(() => setInboxPrefs(DEFAULT_PREFS));
afterEach(() => {
  vi.unstubAllGlobals();
  setInboxPrefs(DEFAULT_PREFS);
});

describe("inbox-model", () => {
  const list = [
    note("a1", { groupKey: "issue:1", updatedAt: "2026-10-03T10:00:00Z" }),
    note("a2", { groupKey: "issue:1", updatedAt: "2026-10-02T10:00:00Z", readAt: "x" }),
    note("b", { groupKey: "issue:2", updatedAt: "2026-10-04T10:00:00Z", isPriority: true }),
    note("c", {
      groupKey: "issue:3",
      updatedAt: "2026-10-01T10:00:00Z",
      category: "mentions",
      actorId: "u-bob",
    }),
  ];

  it("gruppiert nach groupKey und zeigt die neueste Notification", () => {
    const g = groupNotifications(list);
    expect(g.map((x) => x.key)).toEqual(["issue:1", "issue:2", "issue:3"]);
    expect(g[0]!.latest.id).toBe("a1");
    expect(g[0]!.items).toHaveLength(2);
    expect(g[0]!.unread).toBe(1);
  });

  it("sortiert: newest, oldest, priority und unread first", () => {
    const ids = (o: Parameters<typeof buildInbox>[1]) =>
      buildInbox(list, o, NO_FILTERS, "priority").groups.map((g) => g.key);
    expect(ids(DEFAULT_PREFS)).toEqual(["issue:2", "issue:1", "issue:3"]);
    expect(ids({ ...DEFAULT_PREFS, ordering: "oldest" })).toEqual([
      "issue:3",
      "issue:1",
      "issue:2",
    ]);
    expect(ids({ ...DEFAULT_PREFS, ordering: "priority" })[0]).toBe("issue:2");
    expect(ids({ ...DEFAULT_PREFS, unreadFirst: true })[0]).toBe("issue:2");
  });

  it("Priority inbox teilt in Tabs, Filter wirken auf Kategorie und Absender", () => {
    const on = { ...DEFAULT_PREFS, priorityInbox: true };
    expect(buildInbox(list, on, NO_FILTERS, "priority").groups.map((g) => g.key)).toEqual([
      "issue:2",
    ]);
    const other = buildInbox(list, on, NO_FILTERS, "other");
    expect(other.groups).toHaveLength(2);
    expect(other.priorityCount).toBe(1);
    expect(applyFilters(list, { categories: ["mentions"], actorIds: [] })).toHaveLength(1);
    expect(applyFilters(list, { categories: [], actorIds: ["u-bob"] })[0]!.id).toBe("c");
  });

  it("Favorit-Ziel je Bezugsobjekt", () => {
    expect(favoriteTarget(note("x", { issueId: "i1" }))).toEqual({
      type: "issue",
      targetId: "i1",
    });
    expect(favoriteTarget(note("x", { projectId: "p1" }))).toEqual({
      type: "project",
      targetId: "p1",
    });
    expect(favoriteTarget(note("x"))).toBeNull();
  });
});

describe("InboxPage F.09", () => {
  it("zeigt je groupKey eine Zeile mit Zähler und markiert beim Öffnen alle Ungelesenen", async () => {
    const gql = mockGraphql({
      InboxNotifications: () =>
        page([
          note("a1", { groupKey: "issue:1", title: "Login kaputt" }),
          note("a2", { groupKey: "issue:1", title: "Login kaputt" }),
          note("b", { groupKey: "issue:2", title: "Checkout" }),
        ]),
      InboxMarkRead: () => ({ notificationUpdate: { success: true } }),
    });
    await renderWithApp(<InboxPage ws="acme" />, newClient(), "/acme/inbox");
    const user = userEvent.setup();
    await screen.findByText("Checkout");
    expect(screen.getAllByRole("option")).toHaveLength(2);
    expect(screen.getByTestId("group-badge")).toHaveTextContent("+1");
    expect(screen.getByText("2 unread")).toBeInTheDocument();
    await user.click(screen.getAllByText("Login kaputt")[0]!);
    await waitFor(() => expect(gql.of("InboxMarkRead")).toHaveLength(2));
  });

  it("Priority inbox: Tabs Priority und Other aus den Display options", async () => {
    setInboxPrefs({ priorityInbox: true });
    mockGraphql({
      InboxNotifications: () =>
        page([
          note("p", { title: "Dringend", isPriority: true }),
          note("o", { title: "Beiläufig" }),
        ]),
    });
    await renderWithApp(<InboxPage ws="acme" />, newClient(), "/acme/inbox");
    const user = userEvent.setup();
    expect(await screen.findByText("Dringend")).toBeInTheDocument();
    expect(screen.queryByText("Beiläufig")).not.toBeInTheDocument();
    await user.click(screen.getByRole("tab", { name: /Other/ }));
    expect(await screen.findByText("Beiläufig")).toBeInTheDocument();
    expect(screen.queryByText("Dringend")).not.toBeInTheDocument();
  });

  it("Display options im Dialog schalten Priority inbox ein und Ordering um", async () => {
    mockGraphql({
      InboxNotifications: () =>
        page([
          note("n1", {
            title: "Alt",
            updatedAt: "2026-09-01T10:00:00Z",
            createdAt: "2026-09-01T10:00:00Z",
          }),
          note("n2", {
            title: "Neu",
            updatedAt: "2026-10-05T10:00:00Z",
            createdAt: "2026-10-05T10:00:00Z",
          }),
        ]),
    });
    await renderWithApp(<InboxPage ws="acme" />, newClient(), "/acme/inbox");
    const user = userEvent.setup();
    await screen.findByText("Neu");
    const titles = () =>
      [...document.querySelectorAll("[data-notification-id]")].map((o) => o.textContent ?? "");
    expect(titles()[0]).toContain("Neu");
    await user.click(screen.getByRole("button", { name: /Display/ }));
    await user.selectOptions(screen.getByLabelText("Ordering"), "oldest");
    expect(titles()[0]).toContain("Alt");
    await user.click(screen.getByLabelText("Enable priority inbox"));
    expect(screen.getByRole("tab", { name: /Priority/, hidden: true })).toBeInTheDocument();
  });

  it("Filter nach Notification type blendet andere Kategorien aus", async () => {
    mockGraphql({
      InboxNotifications: () =>
        page([
          note("m", { title: "Erwähnung", category: "mentions" }),
          note("c", { title: "Kommentar" }),
        ]),
    });
    await renderWithApp(<InboxPage ws="acme" />, newClient(), "/acme/inbox");
    const user = userEvent.setup();
    await screen.findByText("Erwähnung");
    await user.click(screen.getByRole("button", { name: /^Filter/ }));
    await user.click(screen.getByLabelText("Mentions"));
    const dialog = screen.getByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Clear filters" }));
    await user.click(within(dialog).getByLabelText("Mentions"));
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByText("Kommentar")).not.toBeInTheDocument());
    expect(screen.getByText("Erwähnung")).toBeInTheDocument();
  });

  it("Delete all completed und Delete all senden archiveAll mit Filter", async () => {
    const gql = mockGraphql({
      InboxNotifications: () => page([note("n1")]),
      InboxArchiveAll: () => ({ notificationArchiveAll: { success: true, count: 1 } }),
    });
    await renderWithApp(<InboxPage ws="acme" />, newClient(), "/acme/inbox");
    const user = userEvent.setup();
    await screen.findByText("Note n1");
    await user.click(screen.getByRole("button", { name: "Notification actions" }));
    await user.click(await screen.findByRole("menuitem", { name: "Delete all completed" }));
    await waitFor(() => expect(gql.of("InboxArchiveAll")).toHaveLength(1));
    expect(gql.of("InboxArchiveAll")[0]!.variables).toEqual({ filter: "completed" });
    await user.click(screen.getByRole("button", { name: "Notification actions" }));
    await user.click(await screen.findByRole("menuitem", { name: "Delete all" }));
    await user.click(await screen.findByRole("button", { name: "Delete all" }));
    await waitFor(() => expect(gql.of("InboxArchiveAll")).toHaveLength(2));
    expect(gql.of("InboxArchiveAll")[1]!.variables).toEqual({ filter: "all" });
  });

  it("Kontextmenü: Unsubscribe beendet das Issue-Abo, Snooze und Delete gehen über die Gruppe", async () => {
    const gql = mockGraphql({
      InboxNotifications: () =>
        page([note("n1", { issueIdentifier: "ENG-7", issueId: "i7", title: "Login" })]),
      InboxIssueUnsubscribe: () => ({ issueUnsubscribe: { success: true } }),
      InboxFavorite: () => ({ favoriteCreate: { success: true } }),
      InboxSnooze: () => ({ notificationUpdate: { success: true } }),
      InboxArchive: () => ({ notificationArchive: { success: true } }),
    });
    await renderWithApp(<InboxPage ws="acme" />, newClient(), "/acme/inbox");
    const user = userEvent.setup();
    const row = (await screen.findByText("Login")).closest("[role=option]")!.parentElement!;
    fireEvent.contextMenu(row);
    await user.click(await screen.findByRole("menuitem", { name: "Unsubscribe" }));
    await waitFor(() => expect(gql.of("InboxIssueUnsubscribe")).toHaveLength(1));
    expect(gql.of("InboxIssueUnsubscribe")[0]!.variables).toEqual({ id: "ENG-7" });

    fireEvent.contextMenu(row);
    await user.click(await screen.findByRole("menuitem", { name: "Favorite" }));
    await waitFor(() => expect(gql.of("InboxFavorite")).toHaveLength(1));
    expect(gql.of("InboxFavorite")[0]!.variables).toEqual({
      input: { type: "issue", targetId: "i7" },
    });

    fireEvent.contextMenu(row);
    await user.click(await screen.findByRole("menuitem", { name: "Tomorrow" }));
    await waitFor(() => expect(gql.of("InboxSnooze")).toHaveLength(1));
  });
});
