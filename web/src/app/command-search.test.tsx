import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";
import { commands } from "@/app/commands";
import { ConnectedCommandMenu } from "@/app/command-menu";
import { parseCommandQuery } from "@/app/command-search";
import { buildOpenActions } from "@/app/open-actions";
import { getUiState, setUiState, toggleCommandMenu } from "@/app/ui-state";
import { mockGraphql, newClient, renderWithApp } from "@/features/issues/test-fixtures";
import { attachShortcuts, shortcuts } from "@/lib/shortcuts";

describe("parseCommandQuery", () => {
  it("erkennt die Präfixe aus docs/05 §5.6", () => {
    expect(parseCommandQuery("i login")).toEqual({ prefix: "i", term: "login" });
    expect(parseCommandQuery("p  Web ")).toEqual({ prefix: "p", term: "Web" });
    expect(parseCommandQuery("U alice")).toEqual({ prefix: "u", term: "alice" });
    expect(parseCommandQuery("t ")).toEqual({ prefix: "t", term: "" });
    for (const p of ["l", "f", "d", "n"]) expect(parseCommandQuery(`${p} x`).prefix).toBe(p);
  });
  it("ohne Präfix bleibt der Text der Begriff", () => {
    expect(parseCommandQuery("inbox")).toEqual({ prefix: null, term: "inbox" });
    expect(parseCommandQuery("i")).toEqual({ prefix: null, term: "i" });
    expect(parseCommandQuery("x foo")).toEqual({ prefix: null, term: "x foo" });
  });
});

describe("Command-Menü Objektsuche", () => {
  const offs: Array<() => void> = [];
  afterEach(() => {
    offs.splice(0).forEach((o) => o());
    setUiState({ commandMenuOpen: false, commandMenuQuery: "" });
    shortcuts.reset();
    vi.unstubAllGlobals();
  });

  const issueHit = {
    id: "i1",
    kind: "Issue",
    identifier: "ENG-7",
    title: "Login schlägt fehl",
    teamKey: "ENG",
    status: "Todo",
    archived: false,
    updatedAt: "2026-10-01T10:00:00Z",
    highlights: [],
  };

  it("findet ein Issue per Titel (Quick results) neben den Befehlen", async () => {
    const gql = mockGraphql({
      CommandSearch: () => ({ search: { nodes: [issueHit] } }),
      ViewsFavorites: () => ({ favorites: [] }),
      ViewsList: () => ({ customViews: { nodes: [] } }),
    });
    offs.push(
      commands.register([
        { id: "nav.inbox", title: "Go to Inbox", section: "Navigation", perform: () => {} },
      ]),
    );
    toggleCommandMenu(true);
    await renderWithApp(<ConnectedCommandMenu />, newClient(), "/acme/inbox");
    const user = userEvent.setup();
    await user.type(await screen.findByPlaceholderText(/Type a command/), "Login");
    expect(await screen.findByText("ENG-7 Login schlägt fehl")).toBeInTheDocument();
    expect(screen.getByText("Quick results")).toBeInTheDocument();
    await waitFor(() => expect(gql.of("CommandSearch").length).toBeGreaterThan(0));
    const v = gql.of("CommandSearch").at(-1)!.variables;
    expect(v.types).toEqual(["Issue"]);
    expect(v.first).toBe(5);
  });

  it("Präfix i zeigt nur Issue-Treffer, keine Befehle", async () => {
    mockGraphql({
      CommandSearch: () => ({ search: { nodes: [issueHit] } }),
      ViewsFavorites: () => ({ favorites: [] }),
      ViewsList: () => ({ customViews: { nodes: [] } }),
    });
    offs.push(
      commands.register([
        { id: "nav.inbox", title: "Go to Inbox", section: "Navigation", perform: () => {} },
      ]),
    );
    toggleCommandMenu(true, "i Login");
    await renderWithApp(<ConnectedCommandMenu />, newClient(), "/acme/inbox");
    expect(await screen.findByText("ENG-7 Login schlägt fehl")).toBeInTheDocument();
    expect(screen.queryByText("Go to Inbox")).not.toBeInTheDocument();
    expect(screen.getByPlaceholderText("Search issues…")).toBeInTheDocument();
  });

  it("Präfix t filtert die Teams aus der Session lokal", async () => {
    mockGraphql({
      ViewsFavorites: () => ({ favorites: [] }),
      ViewsList: () => ({ customViews: { nodes: [] } }),
    });
    toggleCommandMenu(true, "t des");
    await renderWithApp(<ConnectedCommandMenu />, newClient(), "/acme/inbox");
    expect(await screen.findByText("Design")).toBeInTheDocument();
    expect(screen.queryByText("Engineering")).not.toBeInTheDocument();
  });

  it("O I öffnet das Menü mit dem Präfix i, Backspace führt zur Befehlsliste zurück", async () => {
    mockGraphql({
      ViewsFavorites: () => ({ favorites: [] }),
      ViewsList: () => ({ customViews: { nodes: [] } }),
    });
    const go = vi.fn();
    const actions = buildOpenActions({ ws: "acme", go });
    expect(actions.map((a) => a.keys)).toEqual(
      expect.arrayContaining([
        "o i",
        "o t",
        "o p",
        "o n",
        "o c",
        "o v",
        "o f",
        "o d",
        "o u",
        "o l",
        "o s",
      ]),
    );
    shortcuts.register({
      id: "open.issue",
      keys: "o i",
      description: "Open issue",
      run: () => actions.find((a) => a.id === "open.issue")!.perform(),
    });
    const detach = attachShortcuts();
    await renderWithApp(<ConnectedCommandMenu />, newClient(), "/acme/inbox");
    const user = userEvent.setup();
    await user.keyboard("oi");
    expect(getUiState().commandMenuOpen).toBe(true);
    expect(getUiState().commandMenuQuery).toBe("i ");
    const input = await screen.findByPlaceholderText("Search issues…");
    expect(input).toHaveValue("i ");
    await user.type(input, "{Backspace}");
    expect(await screen.findByPlaceholderText(/Type a command/)).toBeInTheDocument();
    detach();
  });
});
