import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { mockGraphql, newClient, renderWithApp } from "@/features/issues/test-fixtures";

import { resultPath, SearchPage } from "./search-page";

afterEach(() => vi.unstubAllGlobals());

describe("SearchPage", () => {
  it("searches, filters by type and links to results", async () => {
    const gql = mockGraphql({
      GlobalSearch: (v) => ({
        search: {
          nodes:
            (v.types as string[] | undefined)?.[0] === "Project"
              ? []
              : [
                  {
                    id: "i1",
                    kind: "Issue",
                    identifier: "ENG-7",
                    title: "Checkout fails",
                    teamKey: "ENG",
                    status: "Todo",
                    archived: false,
                    updatedAt: "2026-10-01T00:00:00Z",
                    highlights: [],
                  },
                ],
          hasNextPage: false,
          endCursor: null,
        },
      }),
    });
    await renderWithApp(<SearchPage ws="acme" initialTerm="" />, newClient(), "/acme/search");
    const user = userEvent.setup();
    await user.type(screen.getByLabelText("Search"), "checkout");
    const link = await screen.findByRole("link", { name: /Checkout fails/ });
    expect(link).toHaveAttribute("href", "/acme/issue/ENG-7");
    expect(gql.of("GlobalSearch").at(-1)!.variables).toMatchObject({ term: "checkout" });

    await user.click(screen.getByRole("button", { name: "Projects" }));
    await waitFor(() =>
      expect(gql.of("GlobalSearch").at(-1)!.variables.types).toEqual(["Project"]),
    );
    expect(await screen.findByText(/No results/)).toBeInTheDocument();
  });

  it("maps kinds to paths", () => {
    expect(resultPath("acme", { kind: "Project", id: "x", identifier: "P-ENG-1" })).toBe(
      "/acme/project/P-ENG-1/overview",
    );
    expect(resultPath("acme", { kind: "Document", id: "d1", identifier: null })).toBe(
      "/acme/document/d1",
    );
  });
});
