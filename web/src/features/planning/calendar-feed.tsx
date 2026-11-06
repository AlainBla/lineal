/** Kalender-Abo der Meilensteine: ein geheimer Link je Person (docs/04 §3.7). */
import { CalendarDays } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { errorText } from "@/features/issues/error-text";

import { useCalendarFeedEnabled, useRevokeCalendarFeed, useRotateCalendarFeed } from "./queries";

function FeedDialog({ onClose }: { onClose: () => void }) {
  const enabled = useCalendarFeedEnabled();
  const rotate = useRotateCalendarFeed();
  const revoke = useRevokeCalendarFeed();
  const [url, setUrl] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const failure = enabled.error ?? rotate.error ?? revoke.error;
  const create = () =>
    rotate.mutate(undefined, {
      onSuccess: (next) => {
        setUrl(next);
        setCopied(false);
      },
    });
  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Subscribe in your calendar</DialogTitle>
          <DialogDescription>
            Shows the target dates of all milestones you can see. Anyone with the link can read
            their names and dates, so treat it like a password.
          </DialogDescription>
        </DialogHeader>
        {url && (
          <div className="flex flex-col gap-2">
            <div className="flex gap-2">
              <Input
                readOnly
                aria-label="Calendar link"
                value={url}
                onFocus={(e) => e.target.select()}
              />
              <Button
                variant="outline"
                onClick={() => void navigator.clipboard.writeText(url).then(() => setCopied(true))}
              >
                {copied ? "Copied" : "Copy"}
              </Button>
            </div>
            <p className="text-xs text-muted-foreground">
              This link is shown only once. Add it to your calendar as a subscription by URL.
            </p>
          </div>
        )}
        {!url && enabled.data && (
          <p className="text-sm text-muted-foreground">
            Your calendar link is active. It can't be shown again. A new link replaces it, and
            calendars using the old one stop updating.
          </p>
        )}
        {failure && (
          <p role="alert" className="text-xs text-destructive">
            {errorText(failure)}
          </p>
        )}
        <div className="flex justify-end gap-2">
          {enabled.data && (
            <Button
              variant="ghost"
              disabled={revoke.isPending}
              onClick={() => revoke.mutate(undefined, { onSuccess: () => setUrl(null) })}
            >
              Turn off
            </Button>
          )}
          {!url && (
            <Button disabled={enabled.isLoading || rotate.isPending} onClick={create}>
              {enabled.data ? "Create new link" : "Create link"}
            </Button>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}

export function CalendarFeedButton() {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button variant="outline" size="sm" onClick={() => setOpen(true)}>
        <CalendarDays />
        Calendar
      </Button>
      {open && <FeedDialog onClose={() => setOpen(false)} />}
    </>
  );
}
