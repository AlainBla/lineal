import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";
import { shortcuts } from "@/lib/shortcuts";

import { IssuePickerHost, type PickerRequest } from "./picker-host";
import { PropertyPicker } from "./picker";
import { keys } from "./queries";
import { Toaster } from "./toaster";
import { makeIssue, mockGraphql, newClient, renderWithApp, TEAM_META } from "./test-fixtures";

afterEach(() => {
  vi.unstubAllGlobals();
  shortcuts.reset();
});

const OPTIONS = [
  { id: "a", label: "Alpha" },
  { id: "b", label: "Beta" },
  { id: "c", label: "Gamma" },
];

function Harness({ multi, onSelect }: { multi?: boolean; onSelect: (id: string) => void }) {
  const [open, setOpen] = useState(true);
  return (
    <>
      <span data-testid="open">{String(open)}</span>
      <PropertyPicker
        open={open}
        onOpenChange={setOpen}
        title="Change status…"
        options={OPTIONS}
        selected="b"
        multi={multi}
        onSelect={onSelect}
      />
    </>
  );
}

describe("PropertyPicker per Tastatur", () => {
  it("Ziffer wählt die n-te Option und schließt", async () => {
    const onSelect = vi.fn();
    await renderWithApp(<Harness onSelect={onSelect} />);
    const input = await screen.findByPlaceholderText("Change status…");
    await userEvent.type(input, "3");
    expect(onSelect).toHaveBeenCalledWith("c");
    await waitFor(() => expect(screen.getByTestId("open")).toHaveTextContent("false"));
  });

  it("Tippen filtert, Ziffern gelten dann als Suchtext, Enter wählt die Auswahl", async () => {
    const onSelect = vi.fn();
    await renderWithApp(<Harness onSelect={onSelect} />);
    const input = await screen.findByPlaceholderText("Change status…");
    await userEvent.type(input, "gam");
    expect(screen.queryByText("Alpha")).not.toBeInTheDocument();
    await userEvent.keyboard("{Enter}");
    expect(onSelect).toHaveBeenCalledWith("c");
  });

  it("zeigt das Häkchen am aktuellen Wert und Esc schließt", async () => {
    const onSelect = vi.fn();
    await renderWithApp(<Harness onSelect={onSelect} />);
    await screen.findByPlaceholderText("Change status…");
    const beta = screen.getByText("Beta").closest("[cmdk-item]")!;
    expect(beta.querySelector('[aria-label="Selected"]')).not.toBeNull();
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(screen.getByTestId("open")).toHaveTextContent("false"));
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("Mehrfachauswahl bleibt offen und meldet jede Umschaltung", async () => {
    const onSelect = vi.fn();
    await renderWithApp(<Harness multi onSelect={onSelect} />);
    const input = await screen.findByPlaceholderText("Change status…");
    await userEvent.type(input, "1");
    await userEvent.type(input, "{Backspace}2");
    expect(onSelect.mock.calls.map((c) => c[0])).toEqual(["a", "b"]);
    expect(screen.getByTestId("open")).toHaveTextContent("true");
  });
});

describe("IssuePickerHost", () => {
  function Host({ request }: { request: PickerRequest }) {
    const [req, setReq] = useState<PickerRequest | null>(request);
    return (
      <>
        <IssuePickerHost request={req} onClose={() => setReq(null)} />
        <Toaster />
      </>
    );
  }

  it("Status-Picker ändert den Status optimistisch und sendet issueUpdate", async () => {
    const issue = makeIssue({ id: "i1", identifier: "ENG-1", stateId: "s-todo" });
    const gql = mockGraphql({
      TeamMeta: () => ({
        team: TEAM_META.team,
        workflowStates: TEAM_META.states,
        issueLabels: [],
        cycles: [],
        projects: [],
      }),
      IssueUpdate: (v) => ({
        issueUpdate: {
          success: true,
          issue: {
            ...issue,
            ...(v.input as object),
            stateName: "In Progress",
            stateType: "started",
          },
        },
      }),
    });
    const client = newClient();
    client.setQueryData(keys.issue("ENG-1"), issue);
    await renderWithApp(<Host request={{ kind: "status", issue }} />, client);

    const input = await screen.findByPlaceholderText("Change status…");
    await screen.findByText("In Progress");
    await userEvent.type(input, "3"); // Backlog, Todo, In Progress
    await waitFor(() => expect(gql.of("IssueUpdate")).toHaveLength(1));
    expect(gql.of("IssueUpdate")[0]!.variables).toEqual({
      id: "i1",
      input: { stateId: "s-progress" },
    });
    await waitFor(() =>
      expect(client.getQueryData<typeof issue>(keys.issue("ENG-1"))?.stateName).toBe("In Progress"),
    );
  });

  it("rollt bei Serverfehler zurück und zeigt einen Fehler-Toast", async () => {
    const issue = makeIssue({ id: "i1", identifier: "ENG-1", priority: 3 });
    const gql = mockGraphql({
      TeamMeta: () => ({
        team: TEAM_META.team,
        workflowStates: TEAM_META.states,
        issueLabels: [],
        cycles: [],
        projects: [],
      }),
      IssueUpdate: () => new Error("Priority not allowed"),
    });
    const client = newClient();
    client.setQueryData(keys.issue("ENG-1"), issue);
    await renderWithApp(<Host request={{ kind: "priority", issue }} />, client);
    const input = await screen.findByPlaceholderText("Set priority…");
    await userEvent.type(input, "2"); // Urgent ist Nr. 2 (Nr. 1 = No priority)
    await waitFor(() => expect(gql.of("IssueUpdate")).toHaveLength(1));
    expect(await screen.findByRole("alert")).toHaveTextContent("Priority not allowed");
    await waitFor(() =>
      expect(client.getQueryData<typeof issue>(keys.issue("ENG-1"))?.priority).toBe(3),
    );
  });
});
