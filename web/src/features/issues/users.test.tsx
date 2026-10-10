import { act, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { SESSION_QUERY_KEY } from "@/app/auth";

import { mockGraphql, newClient, renderWithApp, SESSION } from "./test-fixtures";
import { useKnownUsers, useRememberUsers } from "./users";

afterEach(() => vi.unstubAllGlobals());

function Names() {
  const users = useKnownUsers();
  const remember = useRememberUsers();
  return (
    <div>
      <button onClick={() => remember([{ id: "u-x", name: "Xavier (comment author)" }])}>
        remember
      </button>
      <ul aria-label="names">
        {users.map((u) => (
          <li key={u.id}>{u.name}</li>
        ))}
      </ul>
    </div>
  );
}

describe("known users are scoped to the session", () => {
  it("shows none of the previous user's people after another user signs in", async () => {
    mockGraphql({
      KnownUsers: () => ({
        users: [{ id: "u-carol", name: "Carol", displayName: "Carol" }],
      }),
    });
    const client = newClient();
    const first = await renderWithApp(<Names />, client);
    expect(await screen.findByText("Carol")).toBeInTheDocument();
    act(() => screen.getByText("remember").click());
    expect(await screen.findByText("Xavier (comment author)")).toBeInTheDocument();

    // Logout leert den Cache (Shell: client.clear()); danach meldet sich ein anderer User an.
    mockGraphql({
      KnownUsers: () => ({ users: [{ id: "u-erin", name: "Erin", displayName: "Erin" }] }),
    });
    first.unmount();
    client.clear();
    client.setQueryData(SESSION_QUERY_KEY, {
      ...SESSION,
      user: { ...SESSION.user, id: "user-dave", name: "Dave" },
    });
    await renderWithApp(<Names />, client);
    expect(await screen.findByText("Erin")).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByText("Carol")).not.toBeInTheDocument());
    expect(screen.queryByText("Xavier (comment author)")).not.toBeInTheDocument();
  });

  it("keys never mix users even without clearing the cache", async () => {
    mockGraphql({
      KnownUsers: () => ({ users: [{ id: "u-carol", name: "Carol", displayName: "Carol" }] }),
    });
    const client = newClient();
    await renderWithApp(<Names />, client);
    await screen.findByText("Carol");
    mockGraphql({
      KnownUsers: () => ({ users: [{ id: "u-erin", name: "Erin", displayName: "Erin" }] }),
    });
    act(() => {
      client.setQueryData(SESSION_QUERY_KEY, {
        ...SESSION,
        user: { ...SESSION.user, id: "user-dave", name: "Dave" },
      });
    });
    expect(await screen.findByText("Erin")).toBeInTheDocument();
    expect(screen.queryByText("Carol")).not.toBeInTheDocument();
  });
});
