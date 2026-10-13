/**
 * `O`-Sequenzen „Open …“ (docs/08 §3.2, F.12/F.15): öffnen das Command-Menü mit dem passenden
 * Präfix (`O I` → `i `, `O P` → `p ` …); `O C`, `O V`, `O S` springen zu den Übersichten.
 */
import {
  Box,
  FileText,
  Layers,
  RefreshCw,
  Search,
  Settings,
  Star,
  Tag,
  Target,
  User,
  Users,
} from "lucide-react";

import type { Action } from "@/app/commands";
import { workspacePaths } from "@/app/nav";
import { toggleCommandMenu } from "@/app/ui-state";

export function buildOpenActions(args: {
  ws: string;
  go: (path: string) => void;
  guest?: boolean;
}): Action[] {
  const w = workspacePaths(args.ws);
  const open = (prefix: string) => () => toggleCommandMenu(true, `${prefix} `);
  const section = "Navigation";
  const actions: Action[] = [
    { id: "open.issue", title: "Open issue…", keys: "o i", icon: Search, perform: open("i") },
    { id: "open.team", title: "Open team…", keys: "o t", icon: Users, perform: open("t") },
    { id: "open.project", title: "Open project…", keys: "o p", icon: Box, perform: open("p") },
    {
      id: "open.initiative",
      title: "Open initiative…",
      keys: "o n",
      icon: Target,
      perform: open("n"),
    },
    {
      id: "open.cycle",
      title: "Open cycle…",
      keys: "o c",
      icon: RefreshCw,
      perform: () => args.go(w.cycles),
    },
    {
      id: "open.view",
      title: "Open view…",
      keys: "o v",
      icon: Layers,
      perform: () => args.go(w.views),
    },
    { id: "open.favorite", title: "Open favorite…", keys: "o f", icon: Star, perform: open("f") },
    {
      id: "open.document",
      title: "Open document…",
      keys: "o d",
      icon: FileText,
      perform: open("d"),
    },
    { id: "open.user", title: "Open user…", keys: "o u", icon: User, perform: open("u") },
    { id: "open.label", title: "Open issue label…", keys: "o l", icon: Tag, perform: open("l") },
    {
      id: "open.settings",
      title: "Open settings…",
      keys: "o s",
      icon: Settings,
      perform: () => args.go(w.settings),
    },
  ].map((a) => ({ ...a, section }));
  // Gäste sehen weder Initiativen noch Zyklen-Übersichten.
  const hidden = new Set(args.guest ? ["open.initiative", "open.cycle"] : []);
  return actions.filter((a) => !hidden.has(a.id));
}
