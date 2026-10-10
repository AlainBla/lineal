import { QueryClient } from "@tanstack/react-query";
import { describe, expect, it, vi } from "vitest";

import { inboxKeys } from "@/features/inbox/queries";
import { makeIssue } from "@/features/issues/test-fixtures";
import type { IssueList } from "@/features/issues/queries";
import type { Issue } from "@/gql/graphql";

import { applySyncAction, applySyncActions, type SyncAction } from "./sync-actions";

const ENG1 = makeIssue({ id: "i-1", identifier: "ENG-1", title: "Login" });
const ENG2 = makeIssue({ id: "i-2", identifier: "ENG-2", title: "Logout" });

function setup() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const list: IssueList = { issues: [ENG1, ENG2], truncated: false };
  // Listen sind nach Team-Key UND nach UUID gecacht, das Detail nach Kennung und nach UUID.
  qc.setQueryData(["issues", "team", "ENG"], list);
  qc.setQueryData(["issues", "team", "team-eng"], list);
  qc.setQueryData(["issues", "my", "assigned", "user-bob"], { issues: [ENG1], truncated: false });
  qc.setQueryData(["issue", "ENG-1"], ENG1);
  qc.setQueryData(["issue", "i-1"], ENG1);
  return qc;
}

function issueAction(over: Partial<SyncAction> & { data: Record<string, unknown> }): SyncAction {
  return { id: 10, action: "U", modelName: "Issue", modelId: "i-1", ...over };
}

function invalidated(qc: QueryClient, key: readonly unknown[]): boolean {
  return qc.getQueryState(key)?.isInvalidated === true;
}

describe("applySyncAction: Issue", () => {
  it("aktualisiert Listen- und Detail-Cache (Team-Key und UUID) per id", () => {
    const qc = setup();
    applySyncAction(
      qc,
      issueAction({
        data: {
          id: "i-1",
          title: "Login repariert",
          priority: 3,
          updatedAt: "2026-10-10T10:00:00.000Z",
        },
      }),
    );
    for (const key of [
      ["issues", "team", "ENG"],
      ["issues", "team", "team-eng"],
    ]) {
      const list = qc.getQueryData<IssueList>(key)!;
      expect(list.issues[0]).toMatchObject({ id: "i-1", title: "Login repariert", priority: 3 });
      expect(list.issues[1]).toEqual(ENG2); // andere Issues bleiben unberührt
    }
    expect(
      qc.getQueryData<IssueList>(["issues", "my", "assigned", "user-bob"])!.issues[0]!.title,
    ).toBe("Login repariert");
    expect(qc.getQueryData<Issue>(["issue", "ENG-1"])!.title).toBe("Login repariert");
    expect(qc.getQueryData<Issue>(["issue", "i-1"])!.priority).toBe(3);
    // reine Feldänderung: kein erneutes Laden der Listen nötig
    expect(invalidated(qc, ["issues", "team", "ENG"])).toBe(false);
    expect(invalidated(qc, ["issue", "ENG-1"])).toBe(false);
  });

  it("invalidiert Listen, wenn sich die Listen-Zugehörigkeit ändert (z. B. Assignee, Team)", () => {
    const qc = setup();
    applySyncAction(qc, issueAction({ data: { id: "i-1", assigneeId: "user-carol" } }));
    expect(qc.getQueryData<Issue>(["issue", "i-1"])!.assigneeId).toBe("user-carol");
    expect(invalidated(qc, ["issues", "team", "ENG"])).toBe(true);
    expect(invalidated(qc, ["issues", "my", "assigned", "user-bob"])).toBe(true);

    const qc2 = setup();
    applySyncAction(
      qc2,
      issueAction({ data: { id: "i-1", teamId: "team-web", identifier: "WEB-4" } }),
    );
    expect(invalidated(qc2, ["issues", "team", "team-eng"])).toBe(true);
    expect(qc2.getQueryData<Issue>(["issue", "i-1"])!.identifier).toBe("WEB-4");
  });

  it("legt neue Issues über frische Listen an und entfernt archivierte/gelöschte", () => {
    const qc = setup();
    applySyncAction(
      qc,
      issueAction({
        action: "I",
        modelId: "i-3",
        data: { id: "i-3", identifier: "ENG-3", teamId: "team-eng" },
      }),
    );
    expect(invalidated(qc, ["issues", "team", "ENG"])).toBe(true);

    applySyncAction(qc, issueAction({ action: "D", modelId: "i-2", data: {} }));
    expect(qc.getQueryData<IssueList>(["issues", "team", "ENG"])!.issues.map((i) => i.id)).toEqual([
      "i-1",
    ]);
    applySyncAction(qc, issueAction({ action: "A", modelId: "i-1", data: { trashed: true } }));
    expect(qc.getQueryData<IssueList>(["issues", "team", "team-eng"])!.issues).toEqual([]);
    expect(invalidated(qc, ["issue", "ENG-1"])).toBe(true);
  });
});

describe("applySyncActions: andere Modelle", () => {
  it("Notification-Aktionen laden Inbox und Sidebar-Badge neu", () => {
    const qc = setup();
    const spy = vi.spyOn(qc, "invalidateQueries");
    applySyncAction(qc, {
      id: 11,
      action: "U",
      modelName: "Notification",
      modelId: "n-1",
      data: { id: "n-1" },
    });
    expect(spy).toHaveBeenCalledWith({ queryKey: inboxKeys.all });
    expect(spy).toHaveBeenCalledWith({ queryKey: ["sidebar-data"] });
  });

  it("fasst Invalidierungen eines Pakets zusammen", () => {
    const qc = setup();
    const spy = vi.spyOn(qc, "invalidateQueries");
    const n = (id: number): SyncAction => ({
      id,
      action: "I",
      modelName: "Notification",
      modelId: `n-${id}`,
      data: {},
    });
    applySyncActions(qc, [n(1), n(2), n(3)]);
    const keys = spy.mock.calls.map((c) => JSON.stringify(c[0]?.queryKey));
    expect(keys.filter((k) => k === JSON.stringify(inboxKeys.all))).toHaveLength(1);
  });

  it("ordnet Planung, Views und Kommentare den Query-Keys ihrer Bereiche zu", () => {
    const qc = setup();
    const spy = vi.spyOn(qc, "invalidateQueries");
    applySyncActions(qc, [
      { id: 1, action: "U", modelName: "Project", modelId: "p", data: {} },
      { id: 2, action: "U", modelName: "CustomView", modelId: "v", data: {} },
      { id: 3, action: "I", modelName: "Comment", modelId: "c", data: { issueId: "i-1" } },
    ]);
    const keys = spy.mock.calls.map((c) => JSON.stringify(c[0]?.queryKey));
    expect(keys).toContain(JSON.stringify(["planning"]));
    expect(keys).toContain(JSON.stringify(["views"]));
    expect(keys).toContain(JSON.stringify(["comments", "i-1"]));
  });

  it("Mitgliedschaftsänderungen laden alles neu", () => {
    const qc = setup();
    applySyncAction(qc, {
      id: 5,
      action: "D",
      modelName: "TeamMembership",
      modelId: "u",
      data: { teamId: "t", userId: "u" },
    });
    expect(invalidated(qc, ["issues", "team", "ENG"])).toBe(true);
    expect(invalidated(qc, ["issue", "i-1"])).toBe(true);
  });
});
