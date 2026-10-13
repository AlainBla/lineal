import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { mockGraphql, newClient, renderWithApp } from "@/features/issues/test-fixtures";

import { parentInput } from "./document-actions";
import { DocumentsSection } from "./documents-section";

const item = (id: string, title: string, trashed = false) => ({
  id,
  slugId: `slug-${id}`,
  title,
  icon: null,
  color: null,
  url: `/acme/document/slug-${id}`,
  projectId: "p-1",
  issueId: null,
  initiativeId: null,
  cycleId: null,
  teamId: null,
  trashed,
  updatedAt: "2026-10-01T00:00:00Z",
});

afterEach(() => vi.unstubAllGlobals());

describe("parentInput", () => {
  it("liefert genau einen Elternparameter", () => {
    expect(parentInput("project", "Checkout", "")).toEqual({ projectId: "Checkout" });
    expect(parentInput("team", "ENG", "")).toEqual({ teamId: "ENG" });
    expect(parentInput("issue", "ENG-1", "")).toEqual({ issueId: "ENG-1" });
    expect(parentInput("initiative", "Growth", "")).toEqual({ initiativeId: "Growth" });
    expect(parentInput("cycle", "3", "ENG")).toEqual({ cycleId: "3", teamId: "ENG" });
  });
});

describe("DocumentsSection", () => {
  it("listet Dokumente, legt eines aus einem Template an und löscht/stellt wieder her", async () => {
    const created: Record<string, unknown>[] = [];
    const trashed: string[] = [];
    const restored: string[] = [];
    vi.spyOn(window, "confirm").mockReturnValue(true);
    const gql = mockGraphql({
      DocumentList: (v) => ({
        documents: {
          nodes: v.trashed ? [item("d2", "Old notes", true)] : [item("d1", "Launch plan")],
        },
      }),
      DocumentTemplates: () => ({ templates: [{ id: "t-1", name: "Meeting notes" }] }),
      DocumentCreate: (v) => {
        created.push(v.input as Record<string, unknown>);
        return {
          documentCreate: {
            success: true,
            document: { ...item("d3", "Meeting notes"), content: "", contentVersion: 1 },
          },
        };
      },
      DocumentTrash: (v) => {
        trashed.push(String(v.id));
        return { documentDelete: { success: true } };
      },
      DocumentRestoreTrash: (v) => {
        restored.push(String(v.id));
        return { documentUnarchive: { success: true } };
      },
    });
    renderWithApp(
      <DocumentsSection ws="acme" scope={{ projectId: "p-1" }} teamId="team-eng" />,
      newClient(),
    );
    const section = await screen.findByRole("region", { name: "Documents" });
    expect(await within(section).findByRole("link", { name: "Launch plan" })).toHaveAttribute(
      "href",
      "/acme/document/slug-d1",
    );

    const user = userEvent.setup();
    await user.click(within(section).getByRole("button", { name: /New document/ }));
    const form = await within(section).findByRole("form", { name: "New document" });
    await waitFor(() =>
      expect(within(form).getByRole("option", { name: "Meeting notes" })).toBeInTheDocument(),
    );
    expect(gql.of("DocumentTemplates")[0]!.variables).toEqual({ teamId: "team-eng" });
    await user.selectOptions(within(form).getByLabelText("Template"), "t-1");
    await user.click(within(form).getByRole("button", { name: "Create document" }));
    await waitFor(() => expect(created).toHaveLength(1));
    expect(created[0]).toEqual({ projectId: "p-1", templateId: "t-1" });

    await user.click(within(section).getByRole("button", { name: "Delete Launch plan" }));
    await waitFor(() => expect(trashed).toEqual(["d1"]));

    await user.click(within(section).getByRole("button", { name: "Recently deleted" }));
    await user.click(await within(section).findByRole("button", { name: "Restore Old notes" }));
    await waitFor(() => expect(restored).toEqual(["d2"]));
  });

  it("legt ein leeres Dokument mit Titel an Initiative/Team/Issue/Cycle an", async () => {
    const created: Record<string, unknown>[] = [];
    mockGraphql({
      DocumentList: () => ({ documents: { nodes: [] } }),
      DocumentTemplates: () => ({ templates: [] }),
      DocumentCreate: (v) => {
        created.push(v.input as Record<string, unknown>);
        return {
          documentCreate: {
            success: true,
            document: { ...item("d9", "Spec"), content: "", contentVersion: 1 },
          },
        };
      },
    });
    renderWithApp(<DocumentsSection ws="acme" scope={{ cycleId: "c-1" }} />, newClient());
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: /New document/ }));
    await user.type(screen.getByLabelText("Document name"), "Spec");
    await user.click(screen.getByRole("button", { name: "Create document" }));
    await waitFor(() => expect(created).toEqual([{ cycleId: "c-1", title: "Spec" }]));
  });
});
