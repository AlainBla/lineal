/** Teilnehmeranzeige der Echtzeit-Mitbearbeitung: farbige Initialen (Avatare) mit Namen. */
import { useEffect, useState } from "react";

import type { CollabProvider } from "./provider";

export interface Participant {
  clientId: number;
  name: string;
  color: string;
  self: boolean;
}

/** Teilnehmer aus den Awareness-Zuständen (ohne Einträge ohne Namen). */
export function participantsOf(provider: CollabProvider): Participant[] {
  const out: Participant[] = [];
  provider.awareness.getStates().forEach((state, clientId) => {
    const user = (state as { user?: { name?: unknown; color?: unknown } }).user;
    if (!user || typeof user.name !== "string" || typeof user.color !== "string") return;
    out.push({
      clientId,
      name: user.name,
      color: /^#[0-9a-fA-F]{6}$/.test(user.color) ? user.color : "#888888",
      self: clientId === provider.doc.clientID,
    });
  });
  return out.sort((a, b) => a.clientId - b.clientId);
}

function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  return (
    (parts[0]?.[0] ?? "?") + (parts.length > 1 ? (parts.at(-1)?.[0] ?? "") : "")
  ).toUpperCase();
}

export function useParticipants(provider: CollabProvider | null): Participant[] {
  const [list, setList] = useState<Participant[]>([]);
  useEffect(() => {
    if (!provider) return;
    const update = () => setList(participantsOf(provider));
    update();
    provider.awareness.on("change", update);
    return () => provider.awareness.off("change", update);
  }, [provider]);
  return provider ? list : [];
}

export function Presence({ provider }: { provider: CollabProvider | null }) {
  const people = useParticipants(provider);
  if (people.length === 0) return null;
  return (
    <ul aria-label="Participants" className="flex items-center -space-x-1.5">
      {people.map((p) => (
        <li
          key={p.clientId}
          title={p.self ? `${p.name} (you)` : p.name}
          aria-label={p.self ? `${p.name} (you)` : p.name}
          className="flex size-6 items-center justify-center rounded-full border border-background text-[10px] font-semibold text-white"
          style={{ backgroundColor: p.color }}
        >
          {initials(p.name)}
        </li>
      ))}
    </ul>
  );
}

const STATUS_TEXT: Record<string, string> = {
  connecting: "Connecting…",
  connected: "Live",
  disconnected: "Offline – changes sync when reconnected",
  closed: "Disconnected",
  fallback: "Saved automatically",
  rejected: "Change too large – not saved. Reload the page to discard it.",
};

export function ConnectionStatus({
  status,
  canWrite,
}: {
  status: keyof typeof STATUS_TEXT;
  canWrite: boolean;
}) {
  const text = STATUS_TEXT[status] ?? status;
  return (
    <span className="text-xs text-muted-foreground" role="status">
      {status === "connected" && !canWrite ? "Live · read only" : text}
    </span>
  );
}
