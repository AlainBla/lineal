import { getSchema } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";
import { Node as PmNode } from "@tiptap/pm/model";
import { describe, expect, it } from "vitest";

import { findIssueRefs } from "./issue-refs";

const schema = getSchema([StarterKit]);
const p = (...content: unknown[]) => ({ type: "paragraph", content });
const text = (t: string, marks?: unknown[]) => ({
  type: "text",
  text: t,
  ...(marks ? { marks } : {}),
});
const doc = (...content: unknown[]) => PmNode.fromJSON(schema, { type: "doc", content });
const ids = (d: PmNode, keys = ["KOL"]) =>
  findIssueRefs(d, new Set(keys)).map((m) => [m.identifier, d.textBetween(m.from, m.to)]);

describe("findIssueRefs", () => {
  it("findet bekannte Kennungen im Text und in Ticket-Links", () => {
    const d = doc(
      p(
        text("Siehe KOL-80/87, "),
        text("KOL-94", [{ type: "link", attrs: { href: "https://x/kollegeplus/issue/KOL-94" } }]),
        text(" und (KOL-34-Retry)."),
      ),
    );
    expect(ids(d)).toEqual([
      ["KOL-80", "KOL-80"],
      ["KOL-94", "KOL-94"],
      ["KOL-34", "KOL-34"],
    ]);
  });

  it("ignoriert fremde Schlüssel, Code, Teilwörter und fremde Links", () => {
    const d = doc(
      p(text("RUSTSEC-2026-0373 UTF-8 XKOL-1 KOL-1a ")),
      p(text("KOL-5", [{ type: "code" }])),
      { type: "codeBlock", content: [text("KOL-6")] },
      p(text("KOL-7", [{ type: "link", attrs: { href: "https://example.com/x" } }])),
      p(text("KOL-8", [{ type: "link", attrs: { href: "https://x/ws/issue/KOL-9" } }])),
    );
    expect(ids(d)).toEqual([]);
  });

  it("liefert nichts, solange keine Team-Schlüssel bekannt sind", () => {
    expect(ids(doc(p(text("KOL-1"))), [])).toEqual([]);
  });
});
