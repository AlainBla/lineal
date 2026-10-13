import { screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";
import { SESSION_QUERY_KEY } from "@/app/auth";
import { mockGraphql, newClient, renderWithApp, SESSION } from "@/features/issues/test-fixtures";

import { ViewBuilderDialog } from "./view-builder";

afterEach(() => vi.unstubAllGlobals());

const handlers = {
  ViewIssuesList: () => ({
    viewIssues: { nodes: [], hasNextPage: false, endCursor: null },
  }),
};

describe("ViewBuilderDialog Save to (B4.40)", () => {
  it("Gäste sehen nur Personal", async () => {
    mockGraphql(handlers);
    const client = newClient();
    client.setQueryData(SESSION_QUERY_KEY, {
      ...SESSION,
      user: { ...SESSION.user, role: "guest" },
    });
    await renderWithApp(
      <ViewBuilderDialog ws="acme" teamId="team-eng" onClose={() => {}} />,
      client,
    );
    const select = await screen.findByLabelText("Save to");
    expect([...select.querySelectorAll("option")].map((o) => o.textContent)).toEqual(["Personal"]);
    expect(select).toHaveValue("personal");
  });

  it("Mitglieder sehen Workspace und Teams", async () => {
    mockGraphql(handlers);
    await renderWithApp(<ViewBuilderDialog ws="acme" onClose={() => {}} />);
    const select = await screen.findByLabelText("Save to");
    expect([...select.querySelectorAll("option")].map((o) => o.textContent)).toEqual([
      "Personal",
      "Workspace",
      "Engineering",
      "Design",
    ]);
  });
});
