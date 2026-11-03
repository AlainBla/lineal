import { act, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import "@/app/test-polyfills";
import { Topbar } from "@/app/topbar";
import { getUiState, setUiState } from "@/app/ui-state";
import { MyIssuesPage } from "@/features/issues/my-issues-page";
import {
  makeIssue,
  mockGraphql,
  newClient,
  renderWithApp,
  TEAM_META,
} from "@/features/issues/test-fixtures";
import { clearQuickFilter, matchesQuickFilter } from "@/lib/quick-filter";
import { focusSearchTarget } from "@/lib/search-target";
import { attachShortcuts, shortcuts } from "@/lib/shortcuts";

describe("matchesQuickFilter", () => {
  const issue = { identifier: "KOL-160", title: "Login Seite überarbeiten" };
  it("matches the identifier as a case-insensitive prefix", () => {
    expect(matchesQuickFilter(issue, "KOL-16")).toBe(true);
    expect(matchesQuickFilter(issue, "kol-16")).toBe(true);
    expect(matchesQuickFilter({ identifier: "KOL-16", title: "x" }, "kol-16")).toBe(true);
    expect(matchesQuickFilter(issue, "KOL-61")).toBe(false);
    expect(matchesQuickFilter(issue, "OL-16")).toBe(false);
  });
  it("matches a bare number against the number part", () => {
    expect(matchesQuickFilter(issue, "16")).toBe(true);
    expect(matchesQuickFilter(issue, "60")).toBe(false);
  });
  it("matches the title as a case-insensitive substring", () => {
    expect(matchesQuickFilter(issue, "seite")).toBe(true);
    expect(matchesQuickFilter(issue, "ÜBERARB")).toBe(true);
  });
  it("does not match otherwise and treats blank text as match-all", () => {
    expect(matchesQuickFilter(issue, "nope")).toBe(false);
    expect(matchesQuickFilter(issue, "  ")).toBe(true);
  });
});

const mine = [
  makeIssue({ id: "m1", identifier: "ENG-7", title: "Meins", assigneeId: "user-bob" }),
  makeIssue({
    id: "m2",
    identifier: "ENG-8",
    title: "Läuft",
    assigneeId: "user-bob",
    stateId: "s-progress",
    stateType: "started",
    stateName: "In Progress",
  }),
];
const handlers = () => ({
  TeamMeta: () => ({
    team: TEAM_META.team,
    workflowStates: TEAM_META.states,
    issueLabels: TEAM_META.labels,
    cycles: [],
    projects: [],
  }),
  TeamIssues: () => ({ issues: { nodes: mine, hasNextPage: false, endCursor: null } }),
});

let detach: () => void;
beforeEach(() => {
  localStorage.clear();
  detach = attachShortcuts();
  clearQuickFilter();
  setUiState({ commandMenuOpen: false, commandMenuQuery: "" });
});
afterEach(() => {
  detach();
  shortcuts.reset();
  clearQuickFilter();
});

const box = () => screen.getByRole("textbox", { name: "Search or run a command" });

describe("Topbar-Suchfeld", () => {
  it("is the search target for Cmd/Ctrl+F on any page", async () => {
    mockGraphql(handlers());
    await renderWithApp(<Topbar />, newClient(), "/acme/settings");
    expect(box()).toHaveAttribute("data-search-target");
  });

  it("filters the list live, hides empty groups and shows the count; Esc clears", async () => {
    mockGraphql(handlers());
    await renderWithApp(
      <>
        <Topbar />
        <MyIssuesPage ws="acme" tab="assigned" />
      </>,
    );
    await screen.findByText("Meins");
    const user = userEvent.setup();
    await user.click(box());
    await user.keyboard("eng-8");
    await waitFor(() => expect(screen.queryByText("Meins")).toBeNull());
    expect(screen.getByText("Läuft")).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Todo" })).toBeNull();
    expect(screen.getByText("1 issue")).toBeInTheDocument();
    await user.clear(box());
    await user.keyboard("meins");
    expect(await screen.findByText("Meins")).toBeInTheDocument();
    expect(screen.queryByText("Läuft")).toBeNull();
    await user.keyboard("{Escape}");
    expect(await screen.findByText("Läuft")).toBeInTheDocument();
    expect(box()).toHaveValue("");
    expect(box()).not.toHaveFocus();
  });

  it("a route change clears the filter", async () => {
    mockGraphql(handlers());
    const { router } = await renderWithApp(<Topbar />, newClient(), "/acme/inbox");
    const user = userEvent.setup();
    await user.click(box());
    await user.keyboard("abc");
    expect(box()).toHaveValue("abc");
    await act(async () => {
      await router.navigate({ to: "/acme/other" as "/" });
    });
    await waitFor(() => expect(box()).toHaveValue(""));
  });

  it("Enter on a page without an issue list opens the command menu with the text", async () => {
    mockGraphql(handlers());
    await renderWithApp(<Topbar />, newClient(), "/acme/settings");
    const user = userEvent.setup();
    await user.click(box());
    expect(getUiState().commandMenuOpen).toBe(false);
    await user.keyboard("hello{Enter}");
    expect(getUiState().commandMenuOpen).toBe(true);
    expect(getUiState().commandMenuQuery).toBe("hello");
    expect(box()).toHaveValue("");
  });
});

describe("Topbar-Suchfeld auf schmalen Fenstern", () => {
  const field = () => box().closest("label")!;

  it("is collapsed behind a magnifier button that expands and focuses the input", async () => {
    mockGraphql(handlers());
    await renderWithApp(<Topbar />, newClient(), "/acme/settings");
    expect(field()).toHaveClass("hidden", "sm:flex");
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Search" }));
    expect(field()).toHaveClass("flex");
    expect(field()).not.toHaveClass("hidden");
    expect(box()).toHaveFocus();
    expect(screen.queryByRole("button", { name: "Search" })).toBeNull();
  });

  it("Cmd/Ctrl+F expands and focuses the collapsed input", async () => {
    mockGraphql(handlers());
    await renderWithApp(<Topbar />, newClient(), "/acme/settings");
    expect(field()).toHaveClass("hidden");
    act(() => {
      focusSearchTarget();
    });
    expect(field()).not.toHaveClass("hidden");
    expect(box()).toHaveFocus();
  });

  it("Esc clears and collapses; blur with empty text collapses, with text stays open", async () => {
    mockGraphql(handlers());
    await renderWithApp(<Topbar />, newClient(), "/acme/settings");
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Search" }));
    await user.keyboard("abc{Escape}");
    expect(box()).toHaveValue("");
    expect(field()).toHaveClass("hidden");
    await user.click(screen.getByRole("button", { name: "Search" }));
    await user.keyboard("abc");
    await user.tab();
    expect(field()).not.toHaveClass("hidden");
    await user.clear(box());
    box().focus();
    await user.tab();
    expect(field()).toHaveClass("hidden");
  });
});
