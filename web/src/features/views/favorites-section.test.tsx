import { screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";
import { applySyncAction } from "@/lib/store/sync-actions";
import { makeIssue, mockGraphql, newClient, renderWithApp } from "@/features/issues/test-fixtures";

import { FavoritesSection } from "./favorites-section";
import { viewKeys } from "./queries";

afterEach(() => vi.unstubAllGlobals());

const fav = (id: string, over: Record<string, unknown> = {}) => ({
  id,
  type: "issue",
  parentId: null,
  folderName: null,
  sortOrder: 1,
  targetId: "uuid-1",
  predefinedViewType: null,
  predefinedViewTeamId: null,
  ...over,
});

describe("FavoritesSection", () => {
  it("Issue-Favorit zeigt Kennung und Titel statt UUID; Link ist nicht nativ ziehbar (B4.39, B4.02)", async () => {
    mockGraphql({
      ViewsFavorites: () => ({ favorites: [fav("f1")] }),
      ViewsCustomViews: () => ({ customViews: [] }),
      IssueDetail: () => ({
        issue: makeIssue({ id: "uuid-1", identifier: "ENG-120", title: "Checkout überarbeiten" }),
      }),
    });
    await renderWithApp(<FavoritesSection ws="acme" />, newClient(), "/acme/inbox");
    const link = await screen.findByRole("link", { name: /ENG-120 Checkout überarbeiten/ });
    expect(link).toHaveAttribute("href", "/acme/issue/ENG-120");
    expect(link).toHaveAttribute("draggable", "false");
    expect(screen.queryByText("uuid-1")).toBeNull();
  });

  it("Sync-Aktion Favorite lädt die Favoriten neu (B4.03)", async () => {
    const client = newClient();
    const spy = vi.spyOn(client, "invalidateQueries");
    applySyncAction(client, {
      id: 1,
      action: "I",
      modelName: "Favorite",
      modelId: "f1",
      data: {},
    });
    await waitFor(() => expect(spy).toHaveBeenCalledWith({ queryKey: ["views"] }));
    expect(viewKeys.favorites[0]).toBe("views");
  });
});
