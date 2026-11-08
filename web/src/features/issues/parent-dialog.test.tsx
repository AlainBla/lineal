import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";

import type { IssueActions } from "./issue-actions";
import { ParentDialog } from "./parent-dialog";
import { makeIssue, mockGraphql, renderWithApp } from "./test-fixtures";

const ISSUE = makeIssue({ id: "i1", identifier: "ENG-5", title: "Kind" });
const PARENT = makeIssue({ id: "p1", identifier: "ENG-2", title: "Epic" });

function actions(): IssueActions {
  return { setParent: vi.fn() } as unknown as IssueActions;
}

describe("Parent setzen (F.11)", () => {
  it("sucht ein Issue und setzt es per Tastatur als Parent (ohne Issue und Kinder)", async () => {
    const gql = mockGraphql({
      IssueExtras: () => ({
        issue: { id: "i1", children: [makeIssue({ id: "c1", identifier: "ENG-9" })] },
      }),
      IssuePickerSearch: () => ({
        searchIssues: {
          nodes: [
            { issue: { id: "i1", identifier: "ENG-5", title: "Kind", stateType: "unstarted" } },
            { issue: { id: "c1", identifier: "ENG-9", title: "Enkel", stateType: "unstarted" } },
            { issue: { id: "x1", identifier: "ENG-1", title: "Andere", stateType: "started" } },
            { issue: { id: "p1", identifier: "ENG-2", title: "Epic", stateType: "started" } },
          ],
        },
      }),
    });
    const a = actions();
    const onClose = vi.fn();
    await renderWithApp(<ParentDialog issue={ISSUE} actions={a} onClose={onClose} />);
    await waitFor(() => expect(gql.of("IssueExtras").length).toBeGreaterThan(0));
    await userEvent.type(screen.getByLabelText("Search issues"), "Epic");
    await screen.findByRole("option", { name: /ENG-2/ });
    expect(screen.queryByText("ENG-5")).toBeNull();
    await waitFor(() => expect(screen.queryByText("ENG-9")).toBeNull());
    await userEvent.keyboard("{ArrowDown}{Enter}");
    expect(a.setParent).toHaveBeenCalledWith(ISSUE, "p1");
    expect(onClose).toHaveBeenCalled();
    expect(gql.of("IssuePickerSearch")[0]!.variables).toMatchObject({ term: "Epic" });
  });

  it("blendet weitere Issues der Mehrfachauswahl aus", async () => {
    mockGraphql({
      IssueExtras: () => ({ issue: { id: "i1", children: [] } }),
      IssuePickerSearch: () => ({
        searchIssues: {
          nodes: [
            { issue: { id: "s2", identifier: "ENG-6", title: "Zweites", stateType: "unstarted" } },
            { issue: { id: "p1", identifier: "ENG-2", title: "Epic", stateType: "started" } },
          ],
        },
      }),
    });
    await renderWithApp(
      <ParentDialog issue={ISSUE} actions={actions()} alsoExclude={["s2"]} onClose={() => {}} />,
    );
    await userEvent.type(screen.getByLabelText("Search issues"), "E");
    await screen.findByRole("option", { name: /ENG-2/ });
    expect(screen.queryByText("ENG-6")).toBeNull();
  });

  it("entfernt den bestehenden Parent", async () => {
    mockGraphql({ IssueDetail: () => ({ issue: PARENT }) });
    const a = actions();
    const child = makeIssue({ id: "i1", identifier: "ENG-5", parentId: "p1" });
    await renderWithApp(<ParentDialog issue={child} actions={a} onClose={() => {}} />);
    await waitFor(() => expect(screen.getByText("ENG-2 Epic")).toBeInTheDocument());
    await userEvent.click(screen.getByRole("button", { name: "Remove parent" }));
    expect(a.setParent).toHaveBeenCalledWith(child, null);
  });
});
