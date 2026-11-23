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

describe("Dates-Flyout (LIN-81)", () => {
  it("die „Dates“-Zeile trägt das Flyout-Chevron und öffnet kein Inline-Ersetzen", async () => {
    const { user } = setup();
    await user.click(screen.getByRole("button", { name: "Add filter" }));
    const datesRow = await screen.findByRole("menuitem", { name: "Dates" });
    // Flyout-Mechanismus wie die anderen Felder: aria-haspopup="menu".
    expect(datesRow).toHaveAttribute("aria-haspopup", "menu");
    // Solange nicht geöffnet, erscheinen die Datumsfelder nicht in der Hauptliste.
    expect(screen.queryByRole("menuitem", { name: "Created" })).toBeNull();
  });

  it("öffnet per Klick ein Flyout mit den Datumsfeldern", async () => {
    const { user } = setup();
    await user.click(screen.getByRole("button", { name: "Add filter" }));
    await user.click(await screen.findByRole("menuitem", { name: "Dates" }));
    const dates = await screen.findByRole("menu", { name: "Dates" });
    expect(dates).toBeInTheDocument();
    for (const name of ["Due date", "Created", "Updated", "Started", "Completed", "Canceled"]) {
      expect(screen.getByRole("menuitem", { name })).toBeInTheDocument();
    }
    // Die ursprüngliche Kategorienliste bleibt sichtbar (Kontext geht nicht verloren).
    expect(screen.getByRole("menu", { name: "Filter categories" })).toBeInTheDocument();
  });

  it("öffnet per Hover ebenfalls das Datums-Flyout", async () => {
    const { user } = setup();
    await user.click(screen.getByRole("button", { name: "Add filter" }));
    await user.hover(await screen.findByRole("menuitem", { name: "Dates" }));
    expect(await screen.findByRole("menu", { name: "Dates" })).toBeInTheDocument();
  });

  it("Tastatur: Pfeil rechts öffnet die Ebene, Pfeil links schließt sie und fokussiert „Dates“", async () => {
    const { user } = setup();
    await user.click(screen.getByRole("button", { name: "Add filter" }));
    const datesRow = await screen.findByRole("menuitem", { name: "Dates" });
    datesRow.focus();
    await user.keyboard("{ArrowRight}");
    expect(await screen.findByRole("menu", { name: "Dates" })).toBeInTheDocument();
    // Fokus wandert in die Datums-Spalte (erstes Feld).
    await waitFor(() => expect(screen.getByRole("menuitem", { name: "Due date" })).toHaveFocus());
    await user.keyboard("{ArrowLeft}");
    await waitFor(() => expect(screen.queryByRole("menu", { name: "Dates" })).toBeNull());
    expect(datesRow).toHaveFocus();
  });

  it("Tastatur: Enter öffnet die Dates-Ebene ebenfalls", async () => {
    const { user } = setup();
    await user.click(screen.getByRole("button", { name: "Add filter" }));
    const datesRow = await screen.findByRole("menuitem", { name: "Dates" });
    datesRow.focus();
    await user.keyboard("{Enter}");
    expect(await screen.findByRole("menu", { name: "Dates" })).toBeInTheDocument();
  });

  it("Pfeil-links im Werte-Flyout eines Datumsfeldes kehrt zur „Dates“-Spalte zurück", async () => {
    const { user } = setup();
    await user.click(screen.getByRole("button", { name: "Add filter" }));
    await user.click(await screen.findByRole("menuitem", { name: "Dates" }));
    await user.click(await screen.findByRole("menuitem", { name: "Due date" }));
    expect(await screen.findByRole("menu", { name: "Due date values" })).toBeInTheDocument();
    // Der Fokus liegt im Werte-Flyout; Pfeil-links führt zurück in die „Dates“-Spalte,
    // ohne das ganze Menü zu schließen.
    await user.keyboard("{ArrowLeft}");
    await waitFor(() => expect(screen.queryByRole("menu", { name: "Due date values" })).toBeNull());
    expect(screen.getByRole("menu", { name: "Dates" })).toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole("menuitem", { name: "Due date" })).toHaveFocus());
  });

  it("Esc schließt das gesamte Popover – wie bei den anderen Feldern", async () => {
    const { user } = setup();
    // Mit geöffneter „Dates“-Spalte.
    await user.click(screen.getByRole("button", { name: "Add filter" }));
    await user.click(await screen.findByRole("menuitem", { name: "Dates" }));
    expect(await screen.findByRole("menu", { name: "Dates" })).toBeInTheDocument();
    await user.keyboard("{Escape}");
    await waitFor(() =>
      expect(screen.queryByRole("menu", { name: "Filter categories" })).toBeNull(),
    );
    // Parität: bei einem gewöhnlichen Feld schließt Esc genauso das ganze Popover.
    await user.click(screen.getByRole("button", { name: "Add filter" }));
    await user.click(await screen.findByRole("menuitem", { name: "Priority" }));
    expect(await screen.findByRole("menu", { name: "Priority values" })).toBeInTheDocument();
    await user.keyboard("{Escape}");
    await waitFor(() =>
      expect(screen.queryByRole("menu", { name: "Filter categories" })).toBeNull(),
    );
  });

  it("während der Suche gibt es keine „Dates“-Spalte, auch nach vorherigem Hover", async () => {
    const { user } = setup();
    await user.click(screen.getByRole("button", { name: "Add filter" }));
    // „Dates“ als Vorschau öffnen, dann tippen: die Datumsfelder werden flach gesucht.
    await user.hover(await screen.findByRole("menuitem", { name: "Dates" }));
    expect(await screen.findByRole("menu", { name: "Dates" })).toBeInTheDocument();
    await user.type(screen.getByLabelText("Search filters"), "created");
    await waitFor(() => expect(screen.queryByRole("menu", { name: "Dates" })).toBeNull());
    // „Created“ erscheint genau einmal (nicht doppelt in Liste und Spalte).
    expect(screen.getAllByRole("menuitem", { name: "Created" })).toHaveLength(1);
  });

  it("Auswahl eines Datumsfeldes im Flyout erzeugt den passenden Chip", async () => {
    const { user, onChange } = setup();
    await user.click(screen.getByRole("button", { name: "Add filter" }));
    await user.click(await screen.findByRole("menuitem", { name: "Dates" }));
    await user.click(await screen.findByRole("menuitem", { name: "Due date" }));
    // Der Werte-Flyout des Datumsfeldes öffnet sich (dritte Spalte).
    expect(await screen.findByRole("menu", { name: "Due date values" })).toBeInTheDocument();
    await user.click(await screen.findByLabelText("Overdue"));
    await user.click(screen.getByRole("button", { name: "Apply" }));
    const chips = onChange.mock.calls.at(-1)![0] as FilterChip[];
    expect(chips.at(-1)).toMatchObject({ field: "dueDate", values: ["overdue"] });
  });

  it("die Suche findet Datumsfelder weiterhin flach (ohne Dates-Flyout)", async () => {
    const { user } = setup();
    await user.click(screen.getByRole("button", { name: "Add filter" }));
    await user.type(screen.getByLabelText("Search filters"), "created");
    const created = await screen.findByRole("menuitem", { name: "Created" });
    // Direkt in der Kategorienliste, nicht in einer „Dates“-Spalte.
    expect(created).toBeInTheDocument();
    expect(screen.queryByRole("menu", { name: "Dates" })).toBeNull();
    expect(screen.queryByRole("menuitem", { name: "Dates" })).toBeNull();
    // Und öffnet direkt seinen Werte-Flyout.
    await user.click(created);
    expect(await screen.findByRole("menu", { name: "Created values" })).toBeInTheDocument();
  });
});
