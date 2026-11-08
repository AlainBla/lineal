import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { DisplayMenu } from "./display-menu";
import { DEFAULT_DISPLAY } from "./display-options";

function setup() {
  const onChange = vi.fn();
  render(<DisplayMenu options={DEFAULT_DISPLAY} onChange={onChange} showLayout />);
  return { onChange, user: userEvent.setup() };
}

describe("Display-Hover-Submenüs (LIN-44)", () => {
  it("öffnet die Grouping-Optionen per Hover und übernimmt die Auswahl", async () => {
    const { onChange, user } = setup();
    await user.click(screen.getByRole("button", { name: "Display" }));
    await user.hover(await screen.findByRole("button", { name: "Grouping" }));
    // Optionsliste klappt bei Hover auf.
    await user.click(await screen.findByRole("option", { name: "Priority" }));
    expect(onChange).toHaveBeenCalledWith({ grouping: "priority" });
  });

  it("Klick öffnet die Optionen ebenfalls (Touch-Pfad)", async () => {
    const { onChange, user } = setup();
    await user.click(screen.getByRole("button", { name: "Display" }));
    await user.click(await screen.findByRole("button", { name: "Ordering" }));
    await user.click(await screen.findByRole("option", { name: "Priority" }));
    expect(onChange.mock.calls.at(-1)![0]).toMatchObject({ ordering: "priority" });
  });

  it("markiert den aktuellen Wert als ausgewählt", async () => {
    const { user } = setup();
    await user.click(screen.getByRole("button", { name: "Display" }));
    await user.hover(await screen.findByRole("button", { name: "Grouping" }));
    const current = await screen.findByRole("option", { name: "Status" });
    await waitFor(() => expect(current).toHaveAttribute("aria-selected", "true"));
  });
});
