import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { FilterBar, type FilterSources } from "./filter-bar";
import type { FilterChip } from "./filter-model";

const sources: FilterSources = {
  users: [{ id: "u1", name: "Bob" }],
  labels: [{ id: "l1", name: "Bug" }],
  projects: [],
  teams: [{ id: "t1", name: "ENG" }],
  states: [],
  milestones: [],
};

function setup(initial: FilterChip[] = []) {
  const onChange = vi.fn();
  render(<FilterBar chips={initial} onChange={onChange} sources={sources} />);
  return { onChange, user: userEvent.setup() };
}

describe("Filter-Hover-Submenüs (LIN-44)", () => {
  it("öffnet das Werte-Submenü per Hover, ohne es anzuklicken", async () => {
    const { user } = setup();
    await user.click(screen.getByRole("button", { name: "Add filter" }));
    await user.hover(await screen.findByRole("menuitem", { name: "Priority" }));
    // Das Flyout mit den Werten erscheint (nach kurzer Hover-Verzögerung).
    expect(await screen.findByLabelText("Urgent")).toBeInTheDocument();
    expect(screen.getByRole("menu", { name: "Priority values" })).toBeInTheDocument();
  });

  it("Klick auf eine Kategorie öffnet das Submenü ebenfalls (Touch-Pfad)", async () => {
    const { user, onChange } = setup();
    await user.click(screen.getByRole("button", { name: "Add filter" }));
    await user.click(await screen.findByRole("menuitem", { name: "Priority" }));
    await user.click(await screen.findByLabelText("Urgent"));
    await user.click(screen.getByRole("button", { name: "Apply" }));
    const chips = onChange.mock.calls.at(-1)![0] as FilterChip[];
    expect(chips.at(-1)).toMatchObject({ field: "priority", values: ["1"] });
  });

  it("Tastatur: Pfeil rechts öffnet das Submenü", async () => {
    const { user } = setup();
    await user.click(screen.getByRole("button", { name: "Add filter" }));
    const row = await screen.findByRole("menuitem", { name: "Priority" });
    row.focus();
    await user.keyboard("{ArrowRight}");
    expect(await screen.findByRole("menu", { name: "Priority values" })).toBeInTheDocument();
    await user.keyboard("{ArrowLeft}");
    await waitFor(() => expect(screen.queryByRole("menu", { name: "Priority values" })).toBeNull());
  });

  it("wechselt das Submenü beim Überfahren einer anderen Kategorie", async () => {
    const { user } = setup();
    await user.click(screen.getByRole("button", { name: "Add filter" }));
    await user.hover(await screen.findByRole("menuitem", { name: "Priority" }));
    expect(await screen.findByRole("menu", { name: "Priority values" })).toBeInTheDocument();
    await user.hover(await screen.findByRole("menuitem", { name: "Assignee" }));
    expect(await screen.findByRole("menu", { name: "Assignee values" })).toBeInTheDocument();
    expect(screen.queryByRole("menu", { name: "Priority values" })).toBeNull();
  });
});
