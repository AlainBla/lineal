import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { mockGraphql, newClient, renderWithApp } from "@/features/issues/test-fixtures";
import type { Comment } from "@/gql/graphql";

import "@/app/test-polyfills";

import { CommentsSection, groupThreads } from "./comments-section";

// ProseMirror misst Textbereiche; jsdom liefert dafür keine Geometrie.
Range.prototype.getClientRects ??= () =>
  ({
    length: 0,
    item: () => null,
    [Symbol.iterator]: [][Symbol.iterator],
  }) as unknown as DOMRectList;
Range.prototype.getBoundingClientRect ??= () => new DOMRect();
document.elementFromPoint ??= () => null;

const comment = (over: Partial<Comment> & Pick<Comment, "id" | "body">): Comment => ({
  userId: "user-bob",
  user: { id: "user-bob", name: "Bob", displayName: "Bob" },
  issueId: null,
  projectId: "p-1",
  projectMilestoneId: null,
  projectUpdateId: null,
  initiativeId: null,
  initiativeUpdateId: null,
  documentId: null,
  parentId: null,
  quotedText: null,
  anchorStart: null,
  anchorEnd: null,
  anchorState: null,
  editedAt: null,
  resolvedAt: null,
  resolvingUserId: null,
  resolvingCommentId: null,
  reactionData: [],
  createdAt: "2026-10-01T10:00:00Z",
  updatedAt: "2026-10-01T10:00:00Z",
  ...over,
});

afterEach(() => {
  vi.unstubAllGlobals();
  window.location.hash = "";
});

describe("groupThreads", () => {
  it("hängt Antworten an ihre Wurzel und sortiert nach Zeit", () => {
    const a = comment({ id: "a", body: "A", createdAt: "2026-10-01T10:00:00Z" });
    const r = comment({ id: "r", body: "R", parentId: "a", createdAt: "2026-10-01T11:00:00Z" });
    const b = comment({ id: "b", body: "B", createdAt: "2026-10-01T09:00:00Z" });
    const t = groupThreads([r, a, b]);
    expect(t.map((x) => x.root.id)).toEqual(["b", "a"]);
    expect(t[1]!.replies.map((x) => x.id)).toEqual(["r"]);
  });
});

describe("CommentsSection", () => {
  const render = (handlers: Record<string, (v: Record<string, unknown>) => unknown>) => {
    const gql = mockGraphql(handlers as never);
    renderWithApp(
      <CommentsSection target={{ projectId: "p-1" }} title="Project comments" />,
      newClient(),
    );
    return gql;
  };

  it("listet Threads, legt Kommentare und Antworten an", async () => {
    const created: Record<string, unknown>[] = [];
    render({
      TargetComments: () => ({
        comments: {
          nodes: [
            comment({ id: "c1", body: "Looks good" }),
            comment({
              id: "c2",
              body: "Thanks",
              parentId: "c1",
              userId: "user-alice",
              user: { id: "user-alice", name: "Alice", displayName: "Alice" },
            }),
          ],
          pageInfo: { hasNextPage: false, endCursor: null },
        },
      }),
      TargetCommentCreate: (v) => {
        created.push(v.input as Record<string, unknown>);
        return { commentCreate: { success: true } };
      },
    });
    const section = await screen.findByRole("region", { name: "Project comments" });
    expect(await within(section).findByText("Looks good")).toBeInTheDocument();
    expect(within(section).getByText("Thanks")).toBeInTheDocument();

    const user = userEvent.setup();
    await user.click(within(section).getByRole("textbox", { name: "Leave a comment" }));
    await user.keyboard("New top level");
    await user.click(within(section).getByRole("button", { name: "Comment" }));
    await waitFor(() => expect(created).toHaveLength(1));
    expect(created[0]).toMatchObject({ projectId: "p-1", body: "New top level" });

    await user.click(within(section).getByRole("button", { name: "Leave a reply…" }));
    await user.click(within(section).getByRole("textbox", { name: "Leave a reply" }));
    await user.keyboard("A reply");
    await user.click(within(section).getByRole("button", { name: "Reply" }));
    await waitFor(() => expect(created).toHaveLength(2));
    expect(created[1]).toEqual({ parentId: "c1", body: "A reply" });
  });

  it("bearbeitet und löscht nur eigene Kommentare, löst Threads auf", async () => {
    const calls: string[] = [];
    vi.spyOn(window, "confirm").mockReturnValue(true);
    render({
      TargetComments: () => ({
        comments: {
          nodes: [
            comment({ id: "c1", body: "Mine" }),
            comment({
              id: "c2",
              body: "Theirs",
              userId: "user-alice",
              user: { id: "user-alice", name: "Alice", displayName: "Alice" },
            }),
          ],
          pageInfo: { hasNextPage: false, endCursor: null },
        },
      }),
      TargetCommentUpdate: (v) => {
        calls.push(`update:${String(v.id)}:${String((v.input as { body: string }).body)}`);
        return { commentUpdate: { success: true } };
      },
      TargetCommentDelete: (v) => {
        calls.push(`delete:${String(v.id)}`);
        return { commentDelete: { success: true } };
      },
      TargetCommentResolve: (v) => {
        calls.push(`resolve:${String(v.id)}`);
        return { commentResolve: { success: true } };
      },
    });
    const mine = await screen.findByRole("article", { name: "Comment by Bob" });
    const theirs = screen.getByRole("article", { name: "Comment by Alice" });
    expect(within(theirs).queryByRole("button", { name: "Edit" })).toBeNull();
    expect(within(theirs).queryByRole("button", { name: "Delete" })).toBeNull();

    const user = userEvent.setup();
    await user.click(within(mine).getByRole("button", { name: "Edit" }));
    await user.click(within(mine).getByRole("textbox", { name: "Edit comment" }));
    await user.keyboard(" more");
    await user.click(within(mine).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(calls.some((c) => /^update:c1:.*more/.test(c))).toBe(true));

    await user.click(within(mine).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(calls).toContain("delete:c1"));
    await user.click(within(theirs).getByRole("button", { name: "Resolve" }));
    await waitFor(() => expect(calls).toContain("resolve:c2"));
  });

  it("scrollt bei #comment-<id> zum Kommentar", async () => {
    const scroll = vi.fn();
    Element.prototype.scrollIntoView = scroll;
    window.location.hash = "#comment-c2";
    render({
      TargetComments: () => ({
        comments: {
          nodes: [comment({ id: "c1", body: "One" }), comment({ id: "c2", body: "Two" })],
          pageInfo: { hasNextPage: false, endCursor: null },
        },
      }),
    });
    await waitFor(() => expect(scroll).toHaveBeenCalled());
    expect(document.getElementById("comment-c2")).toHaveClass("ring-2");
  });
});
