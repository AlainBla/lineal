import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { mockGraphql, newClient, renderWithApp, TEAM_META } from "@/features/issues/test-fixtures";
import type { IssueLabel } from "@/gql/graphql";

import { WorkspaceLabelsPage } from "./workspace-config-pages";

afterEach(() => vi.unstubAllGlobals());

function label(over: Partial<IssueLabel> & Pick<IssueLabel, "id" | "name">): IssueLabel {
  return { ...TEAM_META.labels[0]!, teamId: null, ...over };
}

const LABELS = [
  label({ id: "l-bug", name: "Bug" }),
  label({ id: "l-defect", name: "Defect" }),
  label({ id: "g-area", name: "Area", isGroup: true }),
];

const base = {
  SettingsWorkspaceLabels: () => ({ issueLabels: LABELS }),
  LabelUsage: () => ({ issueLabelUsage: 7 }),
  LabelMergeTargets: () => ({ issueLabels: LABELS }),
};

async function open() {
  await renderWithApp(<WorkspaceLabelsPage ws="acme" />, newClient(), "/acme/settings/labels");
  await screen.findByText("Defect");
}

describe("Label zusammenführen und löschen", () => {
  it("Merge zeigt die Zahl betroffener Issues und schickt Quelle und Ziel", async () => {
    const gql = mockGraphql({
      ...base,
      LabelMerge: () => ({ issueLabelMerge: { success: true, affectedIssues: 7 } }),
    });
    await open();
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Merge Defect" }));
    const dialog = await screen.findByRole("dialog");
    expect(await within(dialog).findByText(/All 7 issues with this label/)).toBeInTheDocument();
    const submit = within(dialog).getByRole("button", { name: "Merge label" });
    expect(submit).toBeDisabled();
    // Das Label selbst und Gruppen sind keine Ziele.
    const select = within(dialog).getByLabelText("Merge into");
    await waitFor(() =>
      expect(within(select).getByRole("option", { name: "Bug" })).toBeInTheDocument(),
    );
    expect(within(select).queryByRole("option", { name: "Defect" })).toBeNull();
    expect(within(select).queryByRole("option", { name: "Area" })).toBeNull();
    await user.selectOptions(select, "l-bug");
    await user.click(submit);
    await waitFor(() => expect(gql.of("LabelMerge")).toHaveLength(1));
    expect(gql.of("LabelMerge")[0]!.variables).toEqual({ id: "l-defect", targetId: "l-bug" });
  });

  it("Delete nennt die Anzahl und löscht erst nach Klick auf die Bestätigung", async () => {
    const gql = mockGraphql({
      ...base,
      LabelDelete: () => ({ issueLabelDelete: { success: true, affectedIssues: 7 } }),
    });
    await open();
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Delete Defect" }));
    const dialog = await screen.findByRole("dialog");
    expect(await within(dialog).findByText(/used by 7 issues/)).toBeInTheDocument();
    expect(gql.of("LabelDelete")).toHaveLength(0);
    await user.click(within(dialog).getByRole("button", { name: "Delete label" }));
    await waitFor(() => expect(gql.of("LabelDelete")).toHaveLength(1));
    expect(gql.of("LabelDelete")[0]!.variables).toEqual({ id: "l-defect" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("Gruppen lassen sich löschen, aber nicht zusammenführen", async () => {
    mockGraphql(base);
    await open();
    expect(screen.queryByRole("button", { name: "Merge Area" })).toBeNull();
    expect(screen.getByRole("button", { name: "Delete Area" })).toBeInTheDocument();
  });

  it("zeigt Serverfehler im Dialog", async () => {
    mockGraphql({ ...base, LabelDelete: () => new Error("You cannot manage this label.") });
    await open();
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Delete Bug" }));
    const dialog = await screen.findByRole("dialog");
    await within(dialog).findByText(/used by 7 issues/);
    await user.click(within(dialog).getByRole("button", { name: "Delete label" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("You cannot manage");
  });
});
