import { afterEach, describe, expect, it, vi } from "vitest";

import { fetchComments } from "./queries";
import { mockGraphql } from "./test-fixtures";

afterEach(() => vi.unstubAllGlobals());

const comment = (n: number) => ({ id: `c${n}`, body: `Kommentar ${n}` });

describe("fetchComments (B4.14)", () => {
  it("lädt alle Seiten nach, nicht nur die ersten 100", async () => {
    const gql = mockGraphql({
      IssueComments: (v) =>
        v.after
          ? {
              comments: {
                nodes: [comment(101), comment(102), comment(103), comment(104)],
                pageInfo: { hasNextPage: false, endCursor: null },
              },
            }
          : {
              comments: {
                nodes: Array.from({ length: 100 }, (_, i) => comment(i + 1)),
                pageInfo: { hasNextPage: true, endCursor: "cur-100" },
              },
            },
    });
    const all = await fetchComments("i1");
    expect(all).toHaveLength(104);
    expect(all[0]!.id).toBe("c1");
    expect(all[103]!.id).toBe("c104");
    expect(gql.of("IssueComments").map((c) => c.variables.after)).toEqual([null, "cur-100"]);
  });
});
