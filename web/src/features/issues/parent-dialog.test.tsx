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
  it("sucht ein Issue und setzt es als Parent (ohne das Issue selbst)", async () => {
    const gql = mockGraphql({
      ParentSearch: () => ({
        searchIssues: {
          nodes: [
            { issue: { id: "i1", identifier: "ENG-5", title: "Kind", stateType: "unstarted" } },
            { issue: { id: "p1", identifier: "ENG-2", title: "Epic", stateType: "started" } },
          ],
        },
      }),
    });
    const a = actions();
    const onClose = vi.fn();
    await renderWithApp(<ParentDialog issue={ISSUE} actions={a} onClose={onClose} />);
    await userEvent.type(screen.getByLabelText("Search issues"), "Epic");
    await userEvent.click(await screen.findByRole("button", { name: /ENG-2/ }));
    expect(screen.queryByText("ENG-5")).toBeNull();
    expect(a.setParent).toHaveBeenCalledWith(ISSUE, "p1");
    expect(onClose).toHaveBeenCalled();
    expect(gql.of("ParentSearch")[0]!.variables).toMatchObject({ term: "Epic" });
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
