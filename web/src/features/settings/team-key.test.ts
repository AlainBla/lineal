import { describe, expect, it } from "vitest";

import { allowedVisibilities, suggestTeamKey, validateKey } from "./team-key";

describe("suggestTeamKey", () => {
  it("uses the first three letters of a single word", () => {
    expect(suggestTeamKey("Engineering")).toBe("ENG");
    expect(suggestTeamKey("Design")).toBe("DES");
  });
  it("uses initials of up to four words", () => {
    expect(suggestTeamKey("Customer Success Ops")).toBe("CSO");
    expect(suggestTeamKey("a b c d e f")).toBe("ABCD");
  });
  it("transliterates umlauts and strips special characters", () => {
    expect(suggestTeamKey("Über uns")).toBe("UU");
    expect(suggestTeamKey("Ärzte")).toBe("AER");
    expect(suggestTeamKey("Straße")).toBe("STR");
  });
  it("falls back to TEAM and prefixes a leading digit", () => {
    expect(suggestTeamKey("!!!")).toBe("TEAM");
    expect(suggestTeamKey("42 Labs")).toBe("T4L");
  });
  it("avoids keys in use and reserved keys", () => {
    expect(suggestTeamKey("Engineering", ["eng"])).toBe("ENG2");
    expect(suggestTeamKey("Engineering", ["ENG", "ENG2"])).toBe("ENG3");
    expect(suggestTeamKey("P")).toBe("P2");
  });
});

describe("validateKey", () => {
  it("accepts valid keys", () => {
    expect(validateKey("KP")).toBeNull();
    expect(validateKey("ENG2")).toBeNull();
    expect(validateKey("ABCDEFG")).toBeNull();
  });
  it("rejects empty, too long, lowercase, hyphen, leading digit and reserved keys", () => {
    for (const bad of ["", "ABCDEFGH", "eng", "EN-G", "1AB", "P", "I"]) {
      expect(validateKey(bad), bad).not.toBeNull();
    }
  });
});

describe("allowedVisibilities", () => {
  it("offers public/private without a parent", () => {
    expect(allowedVisibilities(undefined)).toEqual(["public", "private"]);
  });
  it("never offers public below a non-public parent (docs/02 §5.2)", () => {
    expect(allowedVisibilities({ visibility: "public" })).toContain("public");
    expect(allowedVisibilities({ visibility: "private" })).toEqual(["restricted", "private"]);
    expect(allowedVisibilities({ visibility: "restricted" })).toEqual(["restricted", "private"]);
  });
});
