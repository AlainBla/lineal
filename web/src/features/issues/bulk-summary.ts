import type { IssueBatchResult } from "./queries";

/** Text des Ergebnis-Toasts (`issueBatchUpdate`). */
export function summarizeBatch(
  total: number,
  result: IssueBatchResult,
  identifierOf: (id: string) => string,
): { message: string; tone?: "error" } {
  const ok = result.issues.length;
  const noun = (n: number) => `${n} issue${n === 1 ? "" : "s"}`;
  if (result.failures.length === 0) return { message: `Updated ${noun(ok)}` };
  const first = result.failures[0]!;
  const detail = `${identifierOf(first.id)}: ${first.message}`;
  const more = result.failures.length > 1 ? ` (+${result.failures.length - 1} more)` : "";
  return {
    message:
      ok === 0
        ? `Could not update ${noun(total)}. ${detail}${more}`
        : `Updated ${ok} of ${total}. ${detail}${more}`,
    tone: "error",
  };
}
