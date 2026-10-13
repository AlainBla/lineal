import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";

import { ActivitySection } from "./comments";
import type { IssueLookup } from "./issue-lookup";
import { makeIssue, mockGraphql, renderWithApp } from "./test-fixtures";
import { resetKnownUsers } from "./users";

vi.mock("./rich-editor", () => ({
  RichEditor: (p: {
    ariaLabel: string;
    value: string;
    editable?: boolean;
    onChange?: (v: string) => void;
  }) =>
    p.editable === false ? (
      <p data-testid="comment-body">{p.value}</p>
    ) : (
      <textarea
        aria-label={p.ariaLabel}
        defaultValue={p.value}
        onChange={(e) => p.onChange?.(e.target.value)}
      />
    ),
}));

const ISSUE = makeIssue({ id: "i1", identifier: "ENG-1" });

function comment(id: string, userId: string, name: string, over: Record<string, unknown> = {}) {
  return {
    id,
    body: `Text ${id}`,
    userId,
    user: { id: userId, name: name.toLowerCase(), displayName: name },
    issueId: "i1",
    parentId: null,
    quotedText: null,
    editedAt: null,
    resolvedAt: null,
    reactionData: [],
    createdAt: "2026-10-02T08:00:00Z",
    updatedAt: "2026-10-02T08:00:00Z",
    ...over,
  };
}

function handlers(nodes: unknown[], extra: Record<string, () => unknown> = {}) {
  return {
    IssueComments: () => ({ comments: { nodes, pageInfo: { hasNextPage: false } } }),
    IssueExtras: () => ({
      issue: { id: "i1", subscribed: false, children: [], relations: {}, history: [] },
    }),
    ...extra,
  };
}

const view = () => <ActivitySection issue={ISSUE} lookup={{} as IssueLookup} />;

describe("Kommentar-Aktionen (F.03)", () => {
  beforeEach(() => resetKnownUsers());

  it("zeigt Bearbeiten/Löschen nur am eigenen Kommentar (Bob), Auflösen an jedem Thread", async () => {
    mockGraphql(handlers([comment("c1", "user-bob", "Bob"), comment("c2", "user-alice", "Alice")]));
    await renderWithApp(view());
    const own = await screen.findByRole("article", { name: "Comment by Bob" });
    expect(within(own).getByRole("button", { name: "Edit comment" })).toBeInTheDocument();
    expect(within(own).getByRole("button", { name: "Delete comment" })).toBeInTheDocument();
    const other = screen.getByRole("article", { name: "Comment by Alice" });
    expect(within(other).queryByRole("button", { name: "Edit comment" })).toBeNull();
    expect(within(other).queryByRole("button", { name: "Delete comment" })).toBeNull();
    expect(within(other).getByRole("button", { name: "Resolve thread" })).toBeInTheDocument();
  });

  it("bearbeitet einen Kommentar", async () => {
    const gql = mockGraphql(
      handlers([comment("c1", "user-bob", "Bob")], {
        CommentUpdate: () => ({
          commentUpdate: { success: true, comment: comment("c1", "user-bob", "Bob") },
        }),
      }),
    );
    await renderWithApp(view());
    const own = await screen.findByRole("article", { name: "Comment by Bob" });
    await userEvent.click(within(own).getByRole("button", { name: "Edit comment" }));
    const box = within(own).getByRole("textbox", { name: "Edit comment" });
    await userEvent.clear(box);
    await userEvent.type(box, "Neu");
    await userEvent.click(within(own).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(gql.of("CommentUpdate")).toHaveLength(1));
    expect(gql.of("CommentUpdate")[0]!.variables).toEqual({ id: "c1", input: { body: "Neu" } });
  });

  it("löscht erst nach Bestätigung", async () => {
    const gql = mockGraphql(
      handlers([comment("c1", "user-bob", "Bob")], {
        CommentDelete: () => ({ commentDelete: { success: true } }),
      }),
    );
    await renderWithApp(view());
    const own = await screen.findByRole("article", { name: "Comment by Bob" });
    await userEvent.click(within(own).getByRole("button", { name: "Delete comment" }));
    expect(gql.of("CommentDelete")).toHaveLength(0);
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(gql.of("CommentDelete")).toHaveLength(1));
    expect(gql.of("CommentDelete")[0]!.variables).toEqual({ id: "c1" });
  });

  it("öffnet einen aufgelösten Thread wieder", async () => {
    const gql = mockGraphql(
      handlers([comment("c2", "user-alice", "Alice", { resolvedAt: "2026-10-03T08:00:00Z" })], {
        CommentResolution: () => ({
          commentUnresolve: { success: true, comment: comment("c2", "user-alice", "Alice") },
        }),
      }),
    );
    await renderWithApp(view());
    const card = await screen.findByRole("article", { name: "Comment by Alice" });
    expect(within(card).getByText("Resolved")).toBeInTheDocument();
    await userEvent.click(within(card).getByRole("button", { name: "Reopen thread" }));
    await waitFor(() => expect(gql.of("CommentResolution")).toHaveLength(1));
    expect(gql.of("CommentResolution")[0]!.variables).toEqual({ id: "c2" });
  });
});
