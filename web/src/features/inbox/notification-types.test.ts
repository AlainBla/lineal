import { describe, expect, it } from "vitest";

import { bundleSummary, targetKind, typeLabel } from "./notification-types";

describe("typeLabel", () => {
  it("kennt die neuen Typen", () => {
    expect(typeLabel("issueUnassignedFromYou")).toBe("Unassigned");
    expect(typeLabel("issueReopened")).toBe("Reopened");
    expect(typeLabel("issueUnblocked")).toBe("Unblocked");
    expect(typeLabel("documentNewComment")).toBe("Document comment");
    expect(typeLabel("projectMilestoneCommentMention")).toBe("Milestone mention");
  });
  it("macht unbekannte Typen lesbar", () => {
    expect(typeLabel("somethingNewHappened")).toBe("Something new happened");
    expect(typeLabel("")).toBe("Notification");
  });
});

describe("bundleSummary", () => {
  it("ohne Bündel: ein Ereignis", () => {
    expect(bundleSummary({ type: "issueNewComment", bundledTypes: [] })).toEqual({
      count: 1,
      labels: ["Comment"],
    });
  });
  it("zählt alle Ereignisse und nennt jede Änderung einmal", () => {
    const s = bundleSummary({
      type: "issueAssignedToYou",
      bundledTypes: ["issueStatusChanged", "issueAssignedToYou", "issueStatusChanged"],
    });
    expect(s.count).toBe(3);
    expect(s.labels).toEqual(["Status changed", "Assigned to you"]);
  });
});

describe("targetKind", () => {
  const base = {
    issueId: null,
    documentId: null,
    initiativeId: null,
    projectId: null,
    projectMilestoneId: null,
    projectUpdateId: null,
    initiativeUpdateId: null,
  };
  it("ordnet Meilensteine unter das Projekt, erkennt aber den Meilenstein", () => {
    expect(targetKind({ ...base, projectId: "p", projectMilestoneId: "m" })).toBe("milestone");
    expect(targetKind({ ...base, projectId: "p" })).toBe("project");
    expect(targetKind({ ...base, documentId: "d" })).toBe("document");
    expect(targetKind({ ...base, initiativeId: "i" })).toBe("initiative");
    expect(targetKind(base)).toBe("system");
  });
});
