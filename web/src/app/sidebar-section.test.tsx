import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { CollapsibleSection } from "./sidebar-section";

afterEach(() => localStorage.clear());

const renderSection = () =>
  render(
    <CollapsibleSection id="workspace" label="Workspace">
      <a href="/x">Issues</a>
    </CollapsibleSection>,
  );

describe("CollapsibleSection", () => {
  it("ist standardmäßig offen und klappt per Klick auf die Überschrift ein", async () => {
    renderSection();
    const button = screen.getByRole("button", { name: "Workspace" });
    expect(button).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("Issues")).toBeInTheDocument();

    await act(async () => fireEvent.click(button));
    expect(button).toHaveAttribute("aria-expanded", "false");
    expect(localStorage.getItem("lineal-sidebar-section:workspace")).toBe("false");
  });

  it("merkt sich den eingeklappten Zustand", () => {
    localStorage.setItem("lineal-sidebar-section:workspace", "false");
    renderSection();
    expect(screen.getByRole("button", { name: "Workspace" })).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    expect(screen.queryByText("Issues")).not.toBeInTheDocument();
  });

  it("speichert beim Aufklappen nichts mehr", async () => {
    localStorage.setItem("lineal-sidebar-section:workspace", "false");
    renderSection();
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Workspace" })));
    expect(screen.getByText("Issues")).toBeInTheDocument();
    expect(localStorage.getItem("lineal-sidebar-section:workspace")).toBeNull();
  });
});
