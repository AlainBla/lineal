import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { mockGraphql, newClient, renderWithApp } from "@/features/issues/test-fixtures";
import { attachShortcuts, shortcuts, useShortcutContext } from "@/lib/shortcuts";

import { RemindHost, requestRemind, useRemindAction } from "./remind";

function Page({ withTarget = true }: { withTarget?: boolean }) {
  useShortcutContext("issue");
  useRemindAction(
    withTarget ? { targetType: "issue", target: "ENG-7", label: "ENG-7" } : null,
    "issue",
  );
  return <RemindHost />;
}

afterEach(() => {
  requestRemind(null);
  shortcuts.reset();
  vi.unstubAllGlobals();
});

describe("Remind me", () => {
  it("Shift+H öffnet den Dialog; Auswahl ruft reminderSet mit Zeitpunkt auf", async () => {
    const gql = mockGraphql({
      MyReminder: () => ({ reminder: null }),
      ReminderSet: (v) => ({
        reminderSet: { success: true, reminder: { id: "r1", remindAt: v.remindAt } },
      }),
    });
    const detach = attachShortcuts();
    await renderWithApp(<Page />, newClient(), "/acme/issue/ENG-7");
    const user = userEvent.setup();
    await user.keyboard("{Shift>}H{/Shift}");
    const item = await screen.findByRole("menuitem", { name: "Tomorrow" });
    await user.click(item);
    await waitFor(() => expect(gql.of("ReminderSet")).toHaveLength(1));
    const v = gql.of("ReminderSet")[0]!.variables;
    expect(v.targetType).toBe("issue");
    expect(v.targetId).toBe("ENG-7");
    expect(new Date(String(v.remindAt)).getTime()).toBeGreaterThan(Date.now());
    detach();
  });

  it("zeigt die bestehende Erinnerung und entfernt sie", async () => {
    const gql = mockGraphql({
      MyReminder: () => ({
        reminder: {
          id: "r1",
          targetType: "issue",
          targetId: "ENG-7",
          remindAt: "2030-01-02T09:00:00.000Z",
        },
      }),
      ReminderSet: () => ({ reminderSet: { success: true, reminder: null } }),
    });
    await renderWithApp(<Page />, newClient(), "/acme/issue/ENG-7");
    requestRemind({ targetType: "issue", target: "ENG-7", label: "ENG-7" });
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Remove reminder" }));
    await waitFor(() => expect(gql.of("ReminderSet")).toHaveLength(1));
    expect(gql.of("ReminderSet")[0]!.variables.remindAt).toBeNull();
  });

  it("ohne Ziel ist Shift+H wirkungslos", async () => {
    mockGraphql({ MyReminder: () => ({ reminder: null }) });
    const detach = attachShortcuts();
    await renderWithApp(<Page withTarget={false} />, newClient(), "/acme/issue/ENG-7");
    const user = userEvent.setup();
    await user.keyboard("{Shift>}H{/Shift}");
    expect(screen.queryByRole("menu", { name: "Remind me" })).not.toBeInTheDocument();
    detach();
  });
});
