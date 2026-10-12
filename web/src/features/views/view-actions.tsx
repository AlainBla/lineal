import { Bell, BellRing, UserRoundCog } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { errorText } from "@/features/issues/error-text";
import { useDirectoryUsers } from "@/features/issues/users";
import type { CustomView } from "@/gql/graphql";

import {
  useSetViewSubscription,
  useUpdateView,
  useViewSubscription,
  VIEW_SUBSCRIPTION_TYPES,
} from "./queries";

/**
 * Glocke im Kopf einer View („Setup … notifications“, docs/05 §4.7): Abo für neue passende Issues
 * und Statuswechsel. Das Auslösen der Benachrichtigung übernimmt der Fan-out (WP32).
 */
export function ViewSubscriptionButton({ view }: { view: CustomView }) {
  const [open, setOpen] = useState(false);
  const sub = useViewSubscription(view.id);
  const set = useSetViewSubscription(view.id);
  const active = sub.data?.active ?? false;
  const current = sub.data?.notificationSubscriptionTypes ?? ["issueAddedToView"];
  const toggleType = (t: string) => {
    const next = current.includes(t) ? current.filter((x) => x !== t) : [...current, t];
    if (next.length > 0) set.mutate({ active: true, types: next });
  };
  return (
    <>
      <Button
        variant="ghost"
        size="icon"
        aria-label={active ? "Notifications on" : "Set up notifications"}
        aria-pressed={active}
        onClick={() => setOpen(true)}
      >
        {active ? <BellRing className="text-primary" /> : <Bell />}
      </Button>
      {open && (
        <Dialog open onOpenChange={(o) => !o && setOpen(false)}>
          <DialogContent className="max-w-sm" aria-describedby={undefined}>
            <DialogTitle>Notifications for {view.name}</DialogTitle>
            <DialogDescription className="sr-only">
              Choose when you are notified about issues in this view.
            </DialogDescription>
            <ul aria-label="Notification types" className="flex flex-col gap-1">
              {VIEW_SUBSCRIPTION_TYPES.map((t) => (
                <li key={t.value}>
                  <label className="flex items-center gap-2 rounded px-1 py-1 text-sm hover:bg-surface-hover">
                    <input
                      type="checkbox"
                      checked={active && current.includes(t.value)}
                      disabled={set.isPending}
                      onChange={() =>
                        active
                          ? toggleType(t.value)
                          : set.mutate({ active: true, types: [t.value] })
                      }
                    />
                    {t.label}
                  </label>
                </li>
              ))}
            </ul>
            {set.isError && (
              <p role="alert" className="text-xs text-destructive">
                {errorText(set.error)}
              </p>
            )}
            <div className="flex justify-end gap-2">
              {active && (
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={set.isPending}
                  onClick={() => set.mutate({ active: false })}
                >
                  Unsubscribe
                </Button>
              )}
              <Button size="sm" onClick={() => setOpen(false)}>
                Done
              </Button>
            </div>
          </DialogContent>
        </Dialog>
      )}
    </>
  );
}

/** Eigentümerwechsel einer geteilten View (docs/05 §4.4: Owner und Admins; Ziel muss die View sehen). */
export function TransferOwnerButton({ view }: { view: CustomView }) {
  const [open, setOpen] = useState(false);
  const [owner, setOwner] = useState("");
  const users = useDirectoryUsers("", undefined, open);
  const update = useUpdateView();
  if (!view.shared || !view.canEdit) return null;
  return (
    <>
      <Button
        variant="ghost"
        size="icon"
        aria-label="Transfer ownership"
        onClick={() => setOpen(true)}
      >
        <UserRoundCog />
      </Button>
      {open && (
        <Dialog open onOpenChange={(o) => !o && setOpen(false)}>
          <DialogContent className="max-w-sm" aria-describedby={undefined}>
            <DialogTitle>Transfer ownership of {view.name}</DialogTitle>
            <DialogDescription className="sr-only">
              The new owner must be able to see this view.
            </DialogDescription>
            <p className="text-xs text-muted-foreground">
              Current owner: {view.ownerName ?? "unknown"}
            </p>
            <select
              aria-label="New owner"
              value={owner}
              onChange={(e) => setOwner(e.target.value)}
              className="h-8 rounded-md border border-input bg-transparent px-2 text-sm"
            >
              <option value="">Select a person…</option>
              {(users.data ?? [])
                .filter((u) => u.id !== view.ownerId)
                .map((u) => (
                  <option key={u.id} value={u.id}>
                    {u.displayName || u.name}
                  </option>
                ))}
            </select>
            {update.isError && (
              <p role="alert" className="text-xs text-destructive">
                {errorText(update.error)}
              </p>
            )}
            <div className="flex justify-end gap-2">
              <Button variant="ghost" size="sm" onClick={() => setOpen(false)}>
                Cancel
              </Button>
              <Button
                size="sm"
                disabled={!owner || update.isPending}
                onClick={() =>
                  update.mutate(
                    { id: view.id, input: { ownerId: owner, expectedVersion: view.version } },
                    { onSuccess: () => setOpen(false) },
                  )
                }
              >
                Transfer
              </Button>
            </div>
          </DialogContent>
        </Dialog>
      )}
    </>
  );
}
