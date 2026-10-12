/** Account → Profile (`/{ws}/settings/account/profile`): Name, Anzeigename, Avatar, Zeitzone. */
import { useRef, useState } from "react";

import { useSession } from "@/app/auth";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { errorText } from "@/features/issues/error-text";
import { ErrorState, ListSkeleton } from "@/features/issues/states";
import { showToast } from "@/features/issues/toast-store";

import {
  avatarProblem,
  timezoneOptions,
  uploadAvatar,
  useUpdateViewer,
  useViewer,
  type Viewer,
  type ViewerUpdate,
} from "./account-queries";
import { workspaceSettingsNav } from "./nav";
import { Field, Section, selectClass, SettingsShell } from "./ui";

const HANDLE_RE = /^[a-z0-9._-]{1,40}$/;

function initials(name: string) {
  return (
    name
      .split(/\s+/)
      .filter(Boolean)
      .slice(0, 2)
      .map((p) => p[0]!.toUpperCase())
      .join("") || "?"
  );
}

function AvatarEditor({ viewer }: { viewer: Viewer }) {
  const update = useUpdateViewer();
  const input = useRef<HTMLInputElement>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const pick = async (file: File | undefined) => {
    if (!file) return;
    const problem = avatarProblem(file);
    if (problem) {
      setError(problem);
      return;
    }
    setError(null);
    setBusy(true);
    try {
      const assetUrl = await uploadAvatar(file);
      await update.mutateAsync({ avatarUrl: assetUrl });
      showToast({ message: "Profile picture updated" });
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
      if (input.current) input.current.value = "";
    }
  };

  return (
    <div className="flex items-center gap-4">
      {viewer.avatarUrl ? (
        <img
          src={viewer.avatarUrl}
          alt="Your profile picture"
          className="h-16 w-16 rounded-full object-cover"
        />
      ) : (
        <span
          aria-label="No profile picture"
          className="flex h-16 w-16 items-center justify-center rounded-full bg-surface-active text-lg font-semibold"
        >
          {initials(viewer.name)}
        </span>
      )}
      <div className="flex flex-col gap-1">
        <div className="flex gap-2">
          <input
            ref={input}
            type="file"
            accept="image/png,image/jpeg,image/gif,image/webp"
            aria-label="Upload profile picture"
            className="hidden"
            onChange={(e) => void pick(e.target.files?.[0])}
          />
          <Button
            size="sm"
            variant="outline"
            disabled={busy}
            onClick={() => input.current?.click()}
          >
            {busy ? "Uploading…" : "Upload picture"}
          </Button>
          {viewer.avatarUrl && (
            <Button
              size="sm"
              variant="ghost"
              disabled={busy || update.isPending}
              onClick={() => update.mutate({ avatarUrl: null })}
            >
              Remove
            </Button>
          )}
        </div>
        <p className="text-xs text-muted-foreground">PNG, JPEG, GIF or WebP, up to 5 MB.</p>
        {error && (
          <p role="alert" className="text-xs text-destructive">
            {error}
          </p>
        )}
      </div>
    </div>
  );
}

function ProfileForm({ viewer }: { viewer: Viewer }) {
  const update = useUpdateViewer();
  const [name, setName] = useState(viewer.name);
  const [displayName, setDisplayName] = useState(viewer.displayName);
  const [timezone, setTimezone] = useState(viewer.timezone ?? "");
  const zones = timezoneOptions();
  const nameError =
    name.trim() === "" || name.length > 255 ? "Enter a name (up to 255 characters)." : null;
  const handleError = HANDLE_RE.test(displayName)
    ? null
    : "Use a-z, 0-9, dot, underscore or dash (up to 40 characters).";
  const dirty =
    name.trim() !== viewer.name ||
    displayName !== viewer.displayName ||
    timezone !== (viewer.timezone ?? "");

  const save = () => {
    const input: ViewerUpdate = {};
    if (name.trim() !== viewer.name) input.name = name.trim();
    if (displayName !== viewer.displayName) input.displayName = displayName;
    if (timezone !== (viewer.timezone ?? "")) input.timezone = timezone === "" ? null : timezone;
    update.mutate(input, { onSuccess: () => showToast({ message: "Profile saved" }) });
  };

  return (
    <form
      className="flex flex-col gap-3"
      aria-label="Profile"
      onSubmit={(e) => {
        e.preventDefault();
        if (!nameError && !handleError && dirty) save();
      }}
    >
      <Field label="Full name">
        <Input aria-label="Full name" value={name} onChange={(e) => setName(e.target.value)} />
        {nameError && (
          <span role="alert" className="text-xs text-destructive">
            {nameError}
          </span>
        )}
      </Field>
      <Field label="Display name (used for @mentions)">
        <Input
          aria-label="Display name"
          value={displayName}
          onChange={(e) => setDisplayName(e.target.value)}
        />
        {handleError && (
          <span role="alert" className="text-xs text-destructive">
            {handleError}
          </span>
        )}
      </Field>
      <Field label="Email">
        <Input aria-label="Email" value={viewer.email} readOnly disabled />
      </Field>
      <Field label="Time zone">
        <select
          aria-label="Time zone"
          className={selectClass}
          value={timezone}
          onChange={(e) => setTimezone(e.target.value)}
        >
          <option value="">Browser default</option>
          {viewer.timezone && !zones.includes(viewer.timezone) && (
            <option value={viewer.timezone}>{viewer.timezone}</option>
          )}
          {zones.map((z) => (
            <option key={z} value={z}>
              {z}
            </option>
          ))}
        </select>
      </Field>
      {update.error != null && (
        <p role="alert" className="text-xs text-destructive">
          {errorText(update.error)}
        </p>
      )}
      <div>
        <Button
          type="submit"
          size="sm"
          disabled={!dirty || !!nameError || !!handleError || update.isPending}
        >
          Save
        </Button>
      </div>
    </form>
  );
}

export function AccountProfilePage({ ws }: { ws: string }) {
  const session = useSession();
  const q = useViewer();
  return (
    <SettingsShell title="Settings" nav={workspaceSettingsNav(ws, session, { account: "profile" })}>
      <Section title="Profile" description="How you appear to others in this workspace.">
        {q.isLoading && <ListSkeleton rows={3} />}
        {q.isError && (
          <ErrorState
            error={q.error}
            title="Could not load your profile"
            onRetry={() => void q.refetch()}
          />
        )}
        {q.data && (
          <>
            <AvatarEditor viewer={q.data} />
            {/* Neu aufbauen, wenn der Server andere Werte liefert (nach dem Speichern). */}
            <ProfileForm key={q.data.id + q.dataUpdatedAt} viewer={q.data} />
          </>
        )}
      </Section>
    </SettingsShell>
  );
}
