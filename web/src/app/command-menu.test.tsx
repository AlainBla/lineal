import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Inbox, Moon } from "lucide-react";
import { afterEach, describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";
import { commands, CommandRegistry, groupBySection } from "@/app/commands";
import { CommandMenu } from "@/app/command-menu";

describe("command registry", () => {
  it("registers, searches and unregisters", () => {
    const reg = new CommandRegistry();
    const off = reg.register([
      { id: "a", title: "Go to Inbox", section: "Navigation", perform: () => {} },
      {
        id: "b",
        title: "Theme: Dark",
        section: "Preferences",
        keywords: ["appearance"],
        perform: () => {},
      },
    ]);
    expect(reg.search("inbox").map((c) => c.id)).toEqual(["a"]);
    expect(reg.search("appearance").map((c) => c.id)).toEqual(["b"]);
    expect(reg.search("").length).toBe(2);
    off();
    expect(reg.list()).toHaveLength(0);
  });

  it("groups by section in first-seen order", () => {
    const groups = groupBySection([
      { id: "1", title: "x", section: "B", perform: () => {} },
      { id: "2", title: "y", section: "A", perform: () => {} },
      { id: "3", title: "z", section: "B", perform: () => {} },
    ]);
    expect(groups.map(([s, c]) => [s, c.length])).toEqual([
      ["B", 2],
      ["A", 1],
    ]);
  });
});

describe("CommandMenu", () => {
  const offs: Array<() => void> = [];
  afterEach(() => offs.splice(0).forEach((off) => off()));

  function setup(open = true) {
    const inbox = vi.fn();
    const dark = vi.fn();
    offs.push(
      commands.register([
        {
          id: "nav.inbox",
          title: "Go to Inbox",
          section: "Navigation",
          shortcut: "g i",
          icon: Inbox,
          perform: inbox,
        },
        { id: "nav.projects", title: "Go to Projects", section: "Navigation", perform: () => {} },
        {
          id: "theme.dark",
          title: "Theme: Dark",
          section: "Preferences",
          icon: Moon,
          perform: dark,
        },
      ]),
    );
    const onOpenChange = vi.fn();
    render(<CommandMenu open={open} onOpenChange={onOpenChange} />);
    return { inbox, dark, onOpenChange };
  }

  it("lists registered commands grouped, with shortcut hints", () => {
    setup();
    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByText("Navigation")).toBeInTheDocument();
    expect(within(dialog).getByText("Preferences")).toBeInTheDocument();
    expect(within(dialog).getAllByRole("option")).toHaveLength(3);
    expect(within(dialog).getByLabelText("G then I")).toBeInTheDocument();
  });

  it("filters while typing and shows an empty state", async () => {
    const user = userEvent.setup();
    setup();
    await user.type(screen.getByPlaceholderText(/type a command/i), "theme");
    const options = screen.getAllByRole("option");
    expect(options).toHaveLength(1);
    expect(options[0]).toHaveTextContent("Theme: Dark");

    await user.clear(screen.getByPlaceholderText(/type a command/i));
    await user.type(screen.getByPlaceholderText(/type a command/i), "zzzzqq");
    expect(screen.queryAllByRole("option")).toHaveLength(0);
    expect(screen.getByText("No results found.")).toBeInTheDocument();
  });

  it("runs the selected command with Enter and closes", async () => {
    const user = userEvent.setup();
    const { inbox, onOpenChange } = setup();
    await user.type(screen.getByPlaceholderText(/type a command/i), "inbox");
    await user.keyboard("{Enter}");
    expect(onOpenChange).toHaveBeenCalledWith(false);
    await Promise.resolve();
    expect(inbox).toHaveBeenCalledTimes(1);
  });

  it("renders nothing when closed", () => {
    setup(false);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
});
