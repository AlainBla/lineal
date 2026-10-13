import { describeError } from "@/lib/graphql";

export function errorText(err: unknown): string {
  return describeError(err);
}
