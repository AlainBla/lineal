/**
 * Klicks auf Links im formatierten Text: Lineal-interne Pfade öffnen im Router (kein Neuladen),
 * fremde Ziele in einem neuen Tab. Unsichere Schemata (javascript: …) werden ignoriert.
 */
export interface LinkNavigator {
  navigate: (opts: { to: string }) => unknown;
}

const SAFE_PROTOCOLS = new Set(["http:", "https:", "mailto:"]);

/** Liefert den Link unter dem Klickziel (auch innerhalb von Chips/Widgets). */
export function linkTarget(target: EventTarget | null): string | null {
  if (!(target instanceof Element)) return null;
  const el = target.closest<HTMLElement>("a[href], [data-href]");
  return el?.getAttribute("href") ?? el?.dataset.href ?? null;
}

/** Öffnet `href`; `true`, wenn der Klick behandelt wurde. */
export function openLink(
  href: string,
  router: LinkNavigator | undefined,
  opts: { newTab?: boolean } = {},
): boolean {
  let url: URL;
  try {
    url = new URL(href, window.location.href);
  } catch {
    return false;
  }
  if (!SAFE_PROTOCOLS.has(url.protocol)) return false;
  const internal = url.origin === window.location.origin;
  if (internal && router && !opts.newTab) {
    void router.navigate({ to: `${url.pathname}${url.search}${url.hash}` });
    return true;
  }
  window.open(url.toString(), "_blank", "noopener,noreferrer");
  return true;
}
