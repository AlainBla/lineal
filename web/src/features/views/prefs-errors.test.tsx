import { QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { mockGraphql, newClient } from "@/features/issues/test-fixtures";
import { clearToasts, getToasts } from "@/features/issues/toast-store";

import {
  usePreferences,
  useResetPreferences,
  useSetDefaultPreferences,
  useUpdatePreferences,
  type PrefTarget,
} from "./queries";

const T: PrefTarget = { viewType: "teamIssues", teamId: "team-eng" };
const prefs = () => ({
  viewPreferences: { values: { grouping: "status" }, hasPersonal: false, hasDefault: false },
});

beforeEach(() => {
  vi.spyOn(console, "error").mockImplementation(() => {});
});
afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  clearToasts();
});

function setup() {
  const client = newClient();
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  return { client, wrapper };
}

describe("Anzeigeoptionen: Fehler sichtbar machen (LIN-10)", () => {
  it("abgelehntes Update: Toast mit Servermeldung, Wert wird zurückgerollt", async () => {
    mockGraphql({
      ViewPrefs: prefs,
      ViewPrefsUpdate: () => new Error("Not allowed"),
    });
    const { wrapper } = setup();
    const { result } = renderHook(() => ({ p: usePreferences(T), u: useUpdatePreferences(T) }), {
      wrapper,
    });
    await waitFor(() => expect(result.current.p.data).toBeDefined());
    await act(async () => {
      result.current.u.mutate({ grouping: "assignee" });
    });
    await waitFor(() => expect(getToasts()).toHaveLength(1));
    expect(getToasts()[0]).toMatchObject({ tone: "error" });
    expect(getToasts()[0]!.message).toContain("Display options could not be saved");
    expect(getToasts()[0]!.message).toContain("Not allowed");
    expect(console.error).toHaveBeenCalled();
    await waitFor(() =>
      expect((result.current.p.data?.values as Record<string, unknown>).grouping).toBe("status"),
    );
  });

  it("abgelehntes Reset und Set-Default zeigen einen Toast", async () => {
    mockGraphql({
      ViewPrefsReset: () => new Error("Reset denied"),
      ViewPrefsSetDefault: () => new Error("Default denied"),
    });
    const { wrapper } = setup();
    const { result } = renderHook(
      () => ({ r: useResetPreferences(T), d: useSetDefaultPreferences(T) }),
      { wrapper },
    );
    await act(async () => result.current.r.mutate());
    await waitFor(() => expect(getToasts()).toHaveLength(1));
    expect(getToasts()[0]!.message).toContain("Reset denied");
    await act(async () => result.current.d.mutate());
    await waitFor(() => expect(getToasts()).toHaveLength(2));
    expect(getToasts()[1]!.message).toContain("Default denied");
  });

  it("Ladefehler: genau ein Toast pro Ziel, Daten bleiben leer (Defaults)", async () => {
    mockGraphql({ ViewPrefs: () => new Error("Boom") });
    const { wrapper } = setup();
    const { result, rerender } = renderHook(() => usePreferences({ ...T, teamId: "team-load" }), {
      wrapper,
    });
    await waitFor(() => expect(getToasts()).toHaveLength(1));
    expect(getToasts()[0]!.message).toContain("Boom");
    rerender();
    await act(async () => {
      await result.current.refetch();
    });
    expect(getToasts()).toHaveLength(1);
    expect(result.current.data).toBeUndefined();
  });
});
