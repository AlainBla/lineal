/**
 * Ticket-Verweise im formatierten Text wie in Linear: `KOL-80` (als Text oder als Link auf
 * `/<ws>/issue/KOL-80`) erscheint als Chip mit Status-Icon, Kennung und Titel und öffnet das Ticket.
 *
 * Projekt-Verweise (`P-KOL-3` oder `[P-KOL-3]`) erscheinen ebenso als Chip und öffnen das Projekt.
 *
 * Reine Darstellung über ProseMirror-Dekorationen: Dokument, Markdown und Yjs-Inhalt bleiben
 * unverändert (kein eigener Node-Typ, der im Server-Schema fehlen würde). Erkannt werden nur
 * Kennungen mit bekanntem Team-Schlüssel, nie in Code.
 */
import { Extension } from "@tiptap/core";
import type { Node as PmNode } from "@tiptap/pm/model";
import { Plugin, PluginKey } from "@tiptap/pm/state";
import { Decoration, DecorationSet } from "@tiptap/pm/view";

export interface IssueRefInfo {
  identifier: string;
  title: string;
  stateType: string;
  /** Statusfarbe (`#RRGGBB`), falls bekannt. */
  color?: string | null;
}

export interface ProjectRefInfo {
  identifier: string;
  name: string;
  /** Projektfarbe (`#RRGGBB`), falls gesetzt. */
  color?: string | null;
}

export interface IssueRefsOptions {
  /** Bekannte Team-Schlüssel (Großbuchstaben); leer = noch nicht geladen. */
  getKeys: () => ReadonlySet<string>;
  /** `undefined` = unbekannt (wird nachgeladen), `null` = existiert nicht/kein Zugriff. */
  lookup: (identifier: string) => IssueRefInfo | null | undefined;
  /** Ziel-URL des Tickets. */
  hrefFor: (identifier: string) => string;
  /** Wie `lookup`, für Projekte. */
  lookupProject: (identifier: string) => ProjectRefInfo | null | undefined;
  /** Ziel-URL des Projekts. */
  projectHrefFor: (identifier: string) => string;
}

export const issueRefsKey = new PluginKey("lineal-issue-refs");

/** Kennung `ABC-123`, nicht Teil eines längeren Worts (`RUSTSEC-2026-0373` scheitert am Schlüssel). */
const IDENT = /(?<![\p{L}\p{N}_-])([A-Z][A-Z0-9]{0,9})-(\d{1,7})(?![\p{L}\p{N}_])/gu;
/** Link auf eine Ticketseite: `…/issue/KOL-80` (optional mit Slug/Query). */
const ISSUE_HREF = /\/issue\/([A-Za-z][A-Za-z0-9]{0,9}-\d{1,7})(?:[/?#]|$)/;

/** Projektkennung `P-KOL-3`; der Schlüssel ist ein Team-Schlüssel. */
const PROJECT_IDENT = /(?<![\p{L}\p{N}_-])P-([A-Z][A-Z0-9]{0,9})-(\d{1,7})(?![\p{L}\p{N}_])/gu;
/** Link auf eine Projektseite: `…/project/P-KOL-3` (optional mit Tab/Query). */
const PROJECT_HREF = /\/project\/(P-[A-Za-z][A-Za-z0-9]{0,9}-\d{1,7})(?:[/?#]|$)/;

export interface ProjectRefMatch {
  from: number;
  to: number;
  identifier: string;
  /** Die Kennung steht in eckigen Klammern; der Chip ersetzt sie optisch. */
  bracketed: boolean;
}

/** Fundstellen von Projektkennungen (ohne Code, Links nur, wenn sie auf dieses Projekt zeigen). */
export function findProjectRefs(doc: PmNode, keys: ReadonlySet<string>): ProjectRefMatch[] {
  const out: ProjectRefMatch[] = [];
  if (keys.size === 0) return out;
  doc.descendants((node, pos, parent) => {
    if (node.type.spec.code) return false;
    if (!node.isText || !node.text) return true;
    if (parent?.type.spec.code) return false;
    if (node.marks.some((m) => m.type.spec.code || m.type.name === "code")) return false;
    const link = node.marks.find((m) => m.type.name === "link");
    const linked = link
      ? PROJECT_HREF.exec(String(link.attrs.href ?? ""))?.[1]?.toUpperCase()
      : null;
    if (link && !linked) return false;
    for (const m of node.text.matchAll(PROJECT_IDENT)) {
      if (!keys.has(m[1]!)) continue;
      if (linked && linked !== m[0]) continue;
      const from = pos + m.index;
      const end = m.index + m[0].length;
      out.push({
        from,
        to: from + m[0].length,
        identifier: m[0],
        bracketed: node.text[m.index - 1] === "[" && node.text[end] === "]",
      });
    }
    return false;
  });
  return out;
}

export interface IssueRefMatch {
  from: number;
  to: number;
  identifier: string;
}

/** Fundstellen im Dokument (ohne Code, Links nur, wenn sie auf genau dieses Ticket zeigen). */
export function findIssueRefs(doc: PmNode, keys: ReadonlySet<string>): IssueRefMatch[] {
  const out: IssueRefMatch[] = [];
  if (keys.size === 0) return out;
  doc.descendants((node, pos, parent) => {
    if (node.type.spec.code) return false;
    if (!node.isText || !node.text) return true;
    if (parent?.type.spec.code) return false;
    if (node.marks.some((m) => m.type.spec.code || m.type.name === "code")) return false;
    const link = node.marks.find((m) => m.type.name === "link");
    const linked = link ? ISSUE_HREF.exec(String(link.attrs.href ?? ""))?.[1]?.toUpperCase() : null;
    if (link && !linked) return false;
    for (const m of node.text.matchAll(IDENT)) {
      const key = m[1]!;
      const identifier = `${key}-${m[2]}`;
      if (!keys.has(key)) continue;
      if (linked && linked !== identifier) continue;
      const from = pos + m.index!;
      out.push({ from, to: from + m[0].length, identifier });
    }
    return false;
  });
  return out;
}

const SAFE_COLOR = /^#[0-9A-Fa-f]{6}$/;
/** Ersatzfarben je Statuskategorie, wenn die Teamfarbe nicht geladen ist. */
const TYPE_COLOR: Record<string, string> = {
  triage: "#fc7840",
  backlog: "#95a2b3",
  unstarted: "#95a2b3",
  started: "#f2c94c",
  completed: "#5e6ad2",
  canceled: "#95a2b3",
  duplicate: "#95a2b3",
};

const SVG_NS = "http://www.w3.org/2000/svg";

/** Status-Icon als DOM (gleiche Formen wie `StatusIcon`), für Widgets außerhalb von React. */
export function statusIconElement(stateType: string, color?: string | null): SVGSVGElement {
  const c = color && SAFE_COLOR.test(color) ? color : (TYPE_COLOR[stateType] ?? "currentColor");
  const svg = document.createElementNS(SVG_NS, "svg");
  svg.setAttribute("width", "14");
  svg.setAttribute("height", "14");
  svg.setAttribute("viewBox", "0 0 14 14");
  svg.setAttribute("fill", "none");
  svg.setAttribute("aria-hidden", "true");
  const add = (tag: string, attrs: Record<string, string>) => {
    const el = document.createElementNS(SVG_NS, tag);
    for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
    svg.appendChild(el);
  };
  const ring = { cx: "7", cy: "7", r: "5.5", stroke: c, "stroke-width": "1.5" };
  switch (stateType) {
    case "backlog":
    case "triage":
      add("circle", { ...ring, "stroke-dasharray": "1.8 1.8" });
      break;
    case "unstarted":
      add("circle", ring);
      break;
    case "started":
      add("circle", ring);
      add("path", { d: "M7 3.5a3.5 3.5 0 0 1 0 7z", fill: c });
      break;
    case "completed":
      add("circle", { cx: "7", cy: "7", r: "6.25", fill: c });
      add("path", {
        d: "m4.4 7.2 1.7 1.7 3.5-3.6",
        stroke: "var(--background)",
        "stroke-width": "1.4",
        "stroke-linecap": "round",
        "stroke-linejoin": "round",
      });
      break;
    default:
      add("circle", { cx: "7", cy: "7", r: "6.25", fill: c });
      add("path", {
        d: "m4.9 4.9 4.2 4.2m0-4.2-4.2 4.2",
        stroke: "var(--background)",
        "stroke-width": "1.4",
        "stroke-linecap": "round",
      });
  }
  return svg;
}

/** Würfel wie das Projekt-Icon der Oberfläche, für Widgets außerhalb von React. */
export function projectIconElement(color?: string | null): SVGSVGElement {
  const c = color && SAFE_COLOR.test(color) ? color : "currentColor";
  const svg = document.createElementNS(SVG_NS, "svg");
  svg.setAttribute("width", "14");
  svg.setAttribute("height", "14");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("fill", "none");
  svg.setAttribute("stroke", c);
  svg.setAttribute("stroke-width", "2");
  svg.setAttribute("stroke-linejoin", "round");
  svg.setAttribute("aria-hidden", "true");
  for (const d of ["M21 8 12 3 3 8v8l9 5 9-5z", "m3 8 9 5 9-5", "M12 13v8"]) {
    const path = document.createElementNS(SVG_NS, "path");
    path.setAttribute("d", d);
    svg.appendChild(path);
  }
  return svg;
}

function projectDecorations(doc: PmNode, opts: IssueRefsOptions): Decoration[] {
  const decos: Decoration[] = [];
  for (const m of findProjectRefs(doc, opts.getKeys())) {
    const info = opts.lookupProject(m.identifier);
    if (!info) continue;
    const href = opts.projectHrefFor(m.identifier);
    const label = `${info.identifier} ${info.name}`;
    if (m.bracketed) {
      decos.push(
        Decoration.inline(m.from - 1, m.from, { class: "issue-ref-bracket" }),
        Decoration.inline(m.to, m.to + 1, { class: "issue-ref-bracket" }),
      );
    }
    decos.push(
      Decoration.widget(
        m.from,
        () => {
          const el = document.createElement("span");
          el.className = "issue-ref issue-ref-start";
          el.contentEditable = "false";
          el.dataset.href = href;
          el.appendChild(projectIconElement(info.color));
          return el;
        },
        { side: -1, ignoreSelection: true, key: `ps:${m.identifier}:${info.color}` },
      ),
      Decoration.inline(m.from, m.to, {
        class: "issue-ref issue-ref-id",
        "data-href": href,
        title: label,
      }),
      Decoration.widget(
        m.to,
        () => {
          const el = document.createElement("span");
          el.className = "issue-ref issue-ref-title";
          el.contentEditable = "false";
          el.dataset.href = href;
          el.textContent = info.name;
          el.title = label;
          return el;
        },
        { side: 1, ignoreSelection: true, key: `pt:${m.identifier}:${info.name}` },
      ),
    );
  }
  return decos;
}

function buildDecorations(doc: PmNode, opts: IssueRefsOptions): DecorationSet {
  const decos: Decoration[] = projectDecorations(doc, opts);
  for (const m of findIssueRefs(doc, opts.getKeys())) {
    const info = opts.lookup(m.identifier);
    if (!info) continue;
    const href = opts.hrefFor(m.identifier);
    const label = `${info.identifier} ${info.title}`;
    decos.push(
      Decoration.widget(
        m.from,
        () => {
          const el = document.createElement("span");
          el.className = "issue-ref issue-ref-start";
          el.contentEditable = "false";
          el.dataset.href = href;
          el.appendChild(statusIconElement(info.stateType, info.color));
          return el;
        },
        {
          side: -1,
          ignoreSelection: true,
          key: `s:${m.identifier}:${info.stateType}:${info.color}`,
        },
      ),
      Decoration.inline(m.from, m.to, {
        class: "issue-ref issue-ref-id",
        "data-href": href,
        title: label,
      }),
      Decoration.widget(
        m.to,
        () => {
          const el = document.createElement("span");
          el.className = "issue-ref issue-ref-title";
          el.contentEditable = "false";
          el.dataset.href = href;
          el.textContent = info.title;
          el.title = label;
          return el;
        },
        { side: 1, ignoreSelection: true, key: `t:${m.identifier}:${info.title}` },
      ),
    );
  }
  return DecorationSet.create(doc, decos);
}

export const IssueRefs = Extension.create<IssueRefsOptions>({
  name: "issueRefs",
  addOptions() {
    return {
      getKeys: () => new Set<string>(),
      lookup: () => null,
      hrefFor: (identifier: string) => `/issue/${identifier}`,
      lookupProject: () => null,
      projectHrefFor: (identifier: string) => `/project/${identifier}/overview`,
    };
  },
  addProseMirrorPlugins() {
    const opts = this.options;
    return [
      new Plugin<DecorationSet>({
        key: issueRefsKey,
        state: {
          init: (_, state) => buildDecorations(state.doc, opts),
          // Neu berechnen bei Textänderung oder wenn nachgeladene Tickets eintreffen (Meta).
          apply: (tr, old, _prev, state) =>
            tr.docChanged || tr.getMeta(issueRefsKey) ? buildDecorations(state.doc, opts) : old,
        },
        props: {
          decorations: (state) => issueRefsKey.getState(state) as DecorationSet,
        },
      }),
    ];
  },
});
