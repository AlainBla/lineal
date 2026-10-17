import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";
import { TooltipProvider } from "@/components/ui/tooltip";

import type { IssueLookup } from "./issue-lookup";
import { IssueCardBody, IssueCardHover } from "./issue-row";
import { makeIssue } from "./test-fixtures";

const lookup: IssueLookup = {
  state: () => undefined,
  label: () => undefined,
  projectName: () => null,
  milestoneName: () => null,
  assigneeName: () => null,
  cycleName: () => null,
  estimationType: () => "notUsed",
};

const issue = makeIssue({ id: "a", identifier: "ENG-1", stateEnteredAt: "2026-10-12T09:00:00Z" });

describe("Verweildauer im Status auf der Board-Karte", () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2026-10-17T10:00:00Z"));
  });
  afterEach(() => vi.useRealTimers());

  it("zeigt die Zeit seit dem letzten Statuswechsel in der Kopfzeile", () => {
    render(<IssueCardBody issue={issue} lookup={lookup} />);
    const body = screen.getByTestId("issue-card-body");
    expect(within(body).getByTestId("card-state-age")).toHaveTextContent("5d");
  });

  it("nennt im Hover das Eintrittsdatum", async () => {
    render(
      <TooltipProvider delayDuration={0}>
        <IssueCardHover issue={issue} lookup={lookup}>
          <div>Karte</div>
        </IssueCardHover>
      </TooltipProvider>,
    );
    await userEvent.hover(screen.getByText("Karte"));
    const hover = await screen.findByTestId("card-hover");
    expect(within(hover).getByText("In status since")).toBeInTheDocument();
    expect(within(hover).getByText("12 Oct (5d)")).toBeInTheDocument();
  });
});
