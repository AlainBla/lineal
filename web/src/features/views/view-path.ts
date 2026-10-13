import { slugify } from "@/features/issues/model";

/** Pfadsegment einer View: `<name-slug>-<slugId>` (docs/08 F12); der Server löst das Segment auf. */
export function viewRef(v: { name: string; slugId: string }): string {
  return `${slugify(v.name)}-${v.slugId}`;
}

export const viewPath = (ws: string, v: { name: string; slugId: string }): string =>
  `/${ws}/view/${viewRef(v)}`;
