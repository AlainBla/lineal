import { describe, expect, it } from "vitest";

import type { IssueHistoryEntry } from "@/gql/graphql";

import { describeHistory, type HistoryResolver } from "./history";

const UUID = /[0-9a-f]{8}-[0-9a-f]{4}-/;

const names = (m: Record<string, string>) => (id: string) => m[id] ?? null;
const resolver: HistoryResolver = {
  user: names({ "u-alice": "Alice", "u-bob": "Bob" }),
  state: names({ "s-todo": "Todo", "s-done": "Done" }),
  label: names({ "l-bug": "Bug" }),
  project: names({ "p-1": "Checkout Relaunch" }),
  cycle: names({ "c-1": "Cycle 4" }),
  team: names({ "t-eng": "Engineering", "t-des": "Design" }),
};

function entry(over: Partial<IssueHistoryEntry>): IssueHistoryEntry {
  return {
    id: "h1",
    actorId: "u-alice",
    actorName: null,
    source: "user",
    changes: {},
    addedLabelIds: [],
    removedLabelIds: [],
    relationChanges: null,
    flags: [],
    createdAt: "2026-10-02T08:00:00Z",
    ...over,
  };
}

const phrases = (over: Partial<IssueHistoryEntry>) =>
  describeHistory(entry(over), resolver).phrases;

describe("describeHistory", () => {
  it("beschreibt Status, Priorität, Assignee, Estimate und Fälligkeit mit Namen", () => {
    const p = phrases({
      changes: {
        stateId: { from: "s-todo", to: "s-done" },
        priority: { from: 0, to: 2 },
        assigneeId: { from: null, to: "u-bob" },
        estimate: { from: 2, to: 3 },
        dueDate: { from: "2026-11-01", to: null },
      },
    });
    expect(p).toEqual([
      "changed status from Todo to Done",
      "changed priority from No priority to High",
      "assigned to Bob",
      "changed estimate from 2 to 3",
      "removed the due date",
    ]);
  });

  it("zeigt Labels (+/−), Projekt, Cycle und Team mit Namen", () => {
    const p = phrases({
      addedLabelIds: ["l-bug", "l-unbekannt"],
      removedLabelIds: ["l-bug"],
      changes: {
        projectId: { from: null, to: "p-1" },
        cycleId: { from: "c-1", to: null },
        teamId: { from: "t-eng", to: "t-des" },
        identifier: { from: "ENG-1", to: "DES-1" },
      },
    });
    expect(p).toEqual([
      "added to project Checkout Relaunch",
      "removed from Cycle 4",
      "moved the issue from Engineering to Design",
      "added labels Bug, a label",
      "removed label Bug",
    ]);
  });

  it("zeigt geschwärzte Ziele (null) neutral und niemals rohe UUIDs", () => {
    const uuid = "6f1d2c3e-aaaa-4bbb-8ccc-0123456789ab";
    const p = phrases({
      changes: {
        teamId: { from: "t-eng", to: null },
        projectId: { from: null, to: null },
        parentId: { from: null, to: null },
        cycleId: { from: uuid, to: "c-1" },
        stateId: { from: uuid, to: "s-done" },
        assigneeId: { from: null, to: uuid },
      },
      addedLabelIds: [uuid],
    });
    expect(p).toContain("moved the issue from Engineering to a private team");
    expect(p).toContain("changed the project (a private item)");
    expect(p).toContain("changed the parent (a private item)");
    expect(p).toContain("moved from another cycle to Cycle 4");
    expect(p).toContain("changed status from another status to Done");
    expect(p).toContain("assigned to another user");
    expect(p.join(" ")).not.toMatch(UUID);
  });

  it("beschreibt Relationen, Flags und Parent", () => {
    const p = phrases({
      flags: ["created", "archived"],
      relationChanges: [
        { type: "blocks", identifier: "ENG-7", action: "added" },
        { type: "duplicateOf", identifier: "ENG-8", action: "removed" },
      ],
      changes: { parentId: { from: null, to: "irgendwas" } },
    });
    expect(p).toEqual([
      "created the issue",
      "archived the issue",
      "made this a sub-issue of a parent issue",
      "marked as blocking ENG-7",
      "removed duplicate relation to ENG-8",
    ]);
  });

  it("macht die Quelle erkennbar und kommt ohne Akteur aus", () => {
    const api = describeHistory(entry({ source: "api", flags: ["archived"] }), resolver);
    expect(api.actor).toBe("Alice");
    expect(api.via).toBe("API");
    const auto = describeHistory(
      entry({ actorId: null, source: "automation", flags: ["autoClosed"] }),
      resolver,
    );
    expect(auto.actor).toBe("Automation");
    expect(auto.via).toBeNull();
    const unknown = describeHistory(entry({ actorId: "u-fremd", flags: ["created"] }), resolver);
    expect(unknown.actor).toBe("Someone");
    // Server liefert den Namen (z. B. für Gäste einer Freigabe, B4.16).
    const named = describeHistory(
      entry({ actorId: "u-fremd", actorName: "Dana", flags: ["created"] }),
      resolver,
    );
    expect(named.actor).toBe("Dana");
    expect(describeHistory(entry({ source: "import", actorId: null }), resolver).actor).toBe(
      "Import",
    );
  });

  it("ergibt keine Sätze, wenn nur nicht darstellbare Felder geändert wurden", () => {
    expect(phrases({ changes: { identifier: { from: "A-1", to: "B-1" } } })).toEqual([]);
  });
});
