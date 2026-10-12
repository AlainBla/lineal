import { createMemoryHistory, createRouter, RouterProvider } from "@tanstack/react-router";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";
import { createQueryClient } from "@/app/query-client";
import { queryClient } from "@/app/query-client";
import { Providers } from "@/app/providers";
import { setUiState } from "@/app/ui-state";
import { breadcrumbFor, teamKeyFromPath, teamPaths, workspacePaths } from "@/app/nav";
import {
  buildTeamTree,
  flattenTeamTree,
  MOCK_SIDEBAR_DATA,
  type SidebarTeam,
} from "@/app/use-sidebar-data";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/cn";
import { shortcuts } from "@/lib/shortcuts";
import { routeTree } from "../routeTree.gen";

const ME = {
  user: { id: "u1", displayName: "Alice", email: "alice@acme.example" },
  organization: { id: "o1", name: "Acme", urlKey: "acme" },
  teams: MOCK_SIDEBAR_DATA.teams.map((t) => ({
    id: t.id,
    key: t.key,
    name: t.name,
    parentId: t.parentId,
    cyclesEnabled: t.cyclesEnabled,
    triageEnabled: t.triageEnabled,
  })),
};

function mockFetch(authed: boolean) {
  const fn = vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url === "/auth/me") {
      return authed
        ? new Response(JSON.stringify(ME), { status: 200 })
        : new Response("{}", { status: 401 });
    }
    if (url.endsWith("/graphql")) {
      return new Response(JSON.stringify({ data: { notificationsUnreadCount: 3 } }), {
        status: 200,
      });
    }
    return new Response("{}", { status: 404 });
  });
  vi.stubGlobal("fetch", fn);
  return fn;
}

async function renderApp(path: string) {
  const router = createRouter({
    routeTree,
    history: createMemoryHistory({ initialEntries: [path] }),
  });
  render(
    <Providers client={queryClient}>
      <RouterProvider router={router} />
    </Providers>,
  );
  return router;
}

beforeEach(() => {
  queryClient.clear();
  localStorage.clear();
  setUiState({
    sidebarOpen: true,
    commandMenuOpen: false,
    shortcutHelpOpen: false,
    createIssueOpen: false,
    createTeamOpen: false,
    detailsOpen: false,
  });
});
afterEach(() => {
  vi.unstubAllGlobals();
  shortcuts.reset();
});

describe("auth guard", () => {
  it("redirects to /login when /auth/me answers 401", async () => {
    mockFetch(false);
    const router = await renderApp("/acme/inbox");
    expect(await screen.findByRole("heading", { name: "Log in to Lineal" })).toBeInTheDocument();
    expect(router.state.location.pathname).toBe("/login");
    expect(router.state.location.search).toMatchObject({ redirect: "/acme/inbox" });
    expect(screen.queryByRole("navigation", { name: "Main" })).not.toBeInTheDocument();
  });

  it("sends signed-in users away from /login", async () => {
    mockFetch(true);
    const router = await renderApp("/login");
    expect(await screen.findByRole("navigation", { name: "Main" })).toBeInTheDocument();
    expect(router.state.location.pathname).toBe("/");
  });
});

describe("shell", () => {
  it("renders sidebar, topbar and the team tree with indented sub-teams", async () => {
    mockFetch(true);
    await renderApp("/");
    const nav = await screen.findByRole("navigation", { name: "Main" });

    for (const name of [
      "Inbox",
      "My issues",
      "Drafts",
      "Overview",
      "Issues",
      "Projects",
      "Initiatives",
      "Views",
      "Workload",
      "Cycles",
    ]) {
      expect(
        within(nav).getAllByRole("link", { name: new RegExp(`^${name}`) }).length,
      ).toBeGreaterThan(0);
    }
    expect(screen.getByRole("button", { name: /workspace menu, acme/i })).toBeInTheDocument();
    expect(screen.getByRole("navigation", { name: "Breadcrumb" })).toBeInTheDocument();

    const eng = await within(nav).findByRole("button", { name: /Engineering/ });
    expect(eng).toHaveAttribute("aria-expanded", "true");
    // Sub-Team WEB hängt eingerückt unter ENG und ist eingeklappt.
    const sub = within(nav).getByRole("list", { name: "Sub-teams of Engineering" });
    expect(within(sub).getByRole("button", { name: /Web/ })).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    expect(within(nav).getByRole("button", { name: /Design/ })).toBeInTheDocument();
  });

  it("shows a placeholder page for routes built by later work packages", async () => {
    mockFetch(true);
    const router = await renderApp("/acme/drafts");
    expect(await screen.findByRole("heading", { name: "Drafts" })).toBeInTheDocument();
    expect(screen.getByText(/not built yet/i)).toBeInTheDocument();
    expect(router.state.location.pathname).toBe("/acme/drafts");
  });

  it("navigates with key sequences (g i, g m)", async () => {
    const user = userEvent.setup();
    mockFetch(true);
    const router = await renderApp("/");
    await screen.findByRole("navigation", { name: "Main" });
    await user.keyboard("gi");
    await waitFor(() => expect(router.state.location.pathname).toBe("/acme/inbox"));
    await user.keyboard("gm");
    await waitFor(() => expect(router.state.location.pathname).toBe("/acme/my-issues/assigned"));
  });

  it("opens the command menu with Ctrl+K and runs a navigation command", async () => {
    const user = userEvent.setup();
    mockFetch(true);
    const router = await renderApp("/");
    await screen.findByRole("navigation", { name: "Main" });
    await user.keyboard("{Control>}k{/Control}");
    const dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByPlaceholderText(/type a command/i), "projects");
    const options = within(dialog).getAllByRole("option");
    expect(options[0]).toHaveTextContent("Go to Projects");
    await user.keyboard("{Enter}");
    await waitFor(() => expect(router.state.location.pathname).toBe("/acme/projects/all"));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  });

  it("toggles the sidebar with [ and exposes it to assistive tech", async () => {
    const user = userEvent.setup();
    mockFetch(true);
    await renderApp("/");
    const aside = (await screen.findByRole("navigation", { name: "Main" })).closest("aside")!;
    expect(aside).toHaveAttribute("data-open", "true");
    await user.keyboard("[[");
    await waitFor(() => expect(aside).toHaveAttribute("data-open", "false"));
    expect(aside).toHaveAttribute("aria-hidden", "true");
  });

  it("switches theme from the command menu", async () => {
    const user = userEvent.setup();
    mockFetch(true);
    await renderApp("/");
    await screen.findByRole("navigation", { name: "Main" });
    await user.keyboard("{Control>}k{/Control}");
    const dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByPlaceholderText(/type a command/i), "theme: light");
    await user.keyboard("{Enter}");
    await waitFor(() => expect(document.documentElement).not.toHaveClass("dark"));
    expect(localStorage.getItem("lineal-theme")).toBe("light");
  });
});

describe("navigation helpers", () => {
  it("builds workspace and team paths per docs/08 §1.3", () => {
    expect(workspacePaths("acme").inbox).toBe("/acme/inbox");
    expect(teamPaths("acme", "ENG").all).toBe("/acme/team/ENG/all");
    expect(teamPaths("acme", "ENG").currentCycle).toBe("/acme/team/ENG/cycle/active");
    expect(teamKeyFromPath("/acme/team/WEB/triage")).toBe("WEB");
    expect(teamKeyFromPath("/acme/inbox")).toBeNull();
    expect(breadcrumbFor("/acme/team/ENG/all")).toEqual(["ENG", "All issues"]);
  });
});

describe("sidebar team tree", () => {
  it("nests sub-teams under their parent", () => {
    const tree = buildTeamTree(MOCK_SIDEBAR_DATA.teams);
    expect(tree.map((n) => n.team.key)).toEqual(["ENG", "DES"]);
    expect(tree[0]?.children.map((n) => n.team.key)).toEqual(["WEB"]);
    expect(flattenTeamTree(tree).map((t) => t.key)).toEqual(["ENG", "WEB", "DES"]);
  });

  it("shows a sub-team as root when the parent is not visible (guest)", () => {
    const web = MOCK_SIDEBAR_DATA.teams.find((t) => t.key === "WEB") as SidebarTeam;
    expect(buildTeamTree([web]).map((n) => n.team.key)).toEqual(["WEB"]);
  });
});

describe("ui basics", () => {
  it("renders a shadcn button", () => {
    render(<Button variant="secondary">Save</Button>);
    expect(screen.getByRole("button", { name: "Save" })).toHaveAttribute("data-slot", "button");
  });

  it("cn merges conflicting tailwind classes", () => {
    expect(cn("px-2", "px-4", undefined)).toBe("px-4");
  });

  it("creates isolated query clients", () => {
    expect(createQueryClient()).not.toBe(queryClient);
  });
});
