import { act, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";

import "@/app/test-polyfills";
import { getUiState, setUiState } from "@/app/ui-state";

import {
  addSavedDraft,
  clearAllCreateDrafts,
  draftScope,
  getCreateDraft,
  listSavedDrafts,
  purgeForeignDrafts,
} from "./create-store";
import { DraftsPage } from "./drafts-page";
import { renderWithApp, SESSION } from "./test-fixtures";

const SCOPE = draftScope(SESSION.user.id, SESSION.workspace.id);

afterEach(() => {
  clearAllCreateDrafts();
  setUiState({ createIssueOpen: false });
});

describe("Drafts-Seite (F.16)", () => {
  it("zeigt nur die Entwürfe des eigenen Nutzers und öffnet einen im Create-Modal", async () => {
    addSavedDraft(SCOPE, { title: "Mein Entwurf", description: "Text **fett**", teamKey: "ENG" });
    addSavedDraft(draftScope("user-alice", SESSION.workspace.id), {
      title: "Fremder Entwurf",
      description: "",
    });
    await renderWithApp(<DraftsPage ws="acme" />);
    const list = await screen.findByRole("list", { name: "Drafts" });
    expect(list).toHaveTextContent("Mein Entwurf");
    expect(list).not.toHaveTextContent("Fremder Entwurf");

    await userEvent.click(screen.getByRole("button", { name: "Open draft Mein Entwurf" }));
    expect(getCreateDraft(SCOPE)?.title).toBe("Mein Entwurf");
    expect(listSavedDrafts(SCOPE)).toHaveLength(0);
    expect(getUiState().createIssueOpen).toBe(true);
    expect(await screen.findByText("No drafts")).toBeInTheDocument();
  });

  it("löscht einen Entwurf; Abmelden und fremder Scope räumen die Liste", async () => {
    addSavedDraft(SCOPE, { title: "Weg damit", description: "" });
    await renderWithApp(<DraftsPage ws="acme" />);
    await userEvent.click(await screen.findByRole("button", { name: "Delete draft Weg damit" }));
    expect(await screen.findByText("No drafts")).toBeInTheDocument();

    addSavedDraft(SCOPE, { title: "A", description: "" });
    purgeForeignDrafts(draftScope("user-alice", SESSION.workspace.id));
    expect(listSavedDrafts(SCOPE)).toHaveLength(0);
    addSavedDraft(SCOPE, { title: "B", description: "" });
    act(() => clearAllCreateDrafts());
    expect(listSavedDrafts(SCOPE)).toHaveLength(0);
    // Leere Entwürfe werden nicht abgelegt.
    expect(addSavedDraft(SCOPE, { title: " ", description: "" })).toBeNull();
  });
});
