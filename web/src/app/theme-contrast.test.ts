import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

import { contrast, parseOklch } from "./contrast";

const css = readFileSync(resolve(__dirname, "theme.css"), "utf8");

/** Tokens eines Blocks (`:root` bzw. `.dark`); `.dark` erbt nichts, hier stehen alle benötigten Farben. */
function tokens(selector: string): Record<string, string> {
  const start = css.indexOf(`${selector} {`);
  const body = css.slice(start, css.indexOf("\n}", start));
  const out: Record<string, string> = {};
  for (const m of body.matchAll(/--([\w-]+):\s*([^;]+);/g)) out[m[1]!] = m[2]!;
  return out;
}

const PAIRS: Array<[string, string]> = [
  ["foreground", "background"],
  ["muted-foreground", "background"],
  ["muted-foreground", "surface"],
  ["muted-foreground", "surface-hover"],
  ["muted-foreground", "surface-active"],
  ["sidebar-foreground", "surface-active"],
  ["muted-foreground", "popover"],
  ["subtle-foreground", "background"],
  ["subtle-foreground", "surface"],
  ["subtle-foreground", "sidebar"],
  ["sidebar-foreground", "sidebar"],
  ["primary-foreground", "primary"],
  ["destructive", "background"],
  ["destructive", "popover"],
];

describe("Theme-Kontrast nach WCAG AA (B4.30)", () => {
  for (const theme of [":root", ".dark"]) {
    const t = tokens(theme);
    for (const [fg, bg] of PAIRS) {
      it(`${theme}: ${fg} auf ${bg} mindestens 4,5:1`, () => {
        const f = parseOklch(t[fg] ?? tokens(":root")[fg] ?? "");
        const b = parseOklch(t[bg] ?? tokens(":root")[bg] ?? "");
        expect(f && b).toBeTruthy();
        expect(contrast(f!, b!)).toBeGreaterThanOrEqual(4.5);
      });
    }
  }
});
