import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";

import { useIssueActions } from "./issue-actions";
import { makeIssue, mockGraphql, renderWithApp } from "./test-fixtures";
import { clearToasts } from "./toast-store";
import { Toaster } from "./toaster";

afterEach(() => {
  vi.unstubAllGlobals();
  clearToasts();
});

function Probe() {
  const actions = useIssueActions();
  const issue = makeIssue({ id: "i1", identifier: "ENG-1" });
  return (
    <button type="button" onClick={() => actions.setPriority(issue, 2)}>
      set
    </button>
  );
}

describe("Fehlgeschlagene Mutation (B4.06)", () => {
  it("zeigt einen Fehler-Toast, auch wenn der Aufruf keine eigenen Callbacks hat", async () => {
    mockGraphql({ IssueUpdate: () => new Error("Too many requests") });
    await renderWithApp(
      <>
        <Probe />
        <Toaster />
      </>,
    );
    await userEvent.click(screen.getByRole("button", { name: "set" }));
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("Too many requests"));
  });
});
