import { beforeAll, describe, expect, it } from "vitest";

import { type EmojiData, iconEmoji, loadEmoji, searchEmoji } from "./emoji";

let data: EmojiData;
beforeAll(async () => {
  data = await loadEmoji();
});

describe("iconEmoji", () => {
  it("wandelt Linear-Kurzcodes in Emojis", () => {
    expect(iconEmoji(":bar_chart:", data)).toBe("📊");
    expect(iconEmoji(":speech_balloon:", data)).toBe("💬");
  });
  it("lässt Emojis stehen und verwirft Symbolnamen und Unbekanntes", () => {
    expect(iconEmoji("🚀", data)).toBe("🚀");
    expect(iconEmoji("👩‍💻", data)).toBe("👩‍💻");
    expect(iconEmoji("Rocket", data)).toBeNull();
    expect(iconEmoji(":gibt_es_nicht:", data)).toBeNull();
    expect(iconEmoji("", data)).toBeNull();
    expect(iconEmoji(null, data)).toBeNull();
  });
  it("zeigt den Kurzcode, solange die Daten fehlen", () => {
    expect(iconEmoji(":bar_chart:", null)).toBe(":bar_chart:");
  });
});

describe("searchEmoji", () => {
  it("bevorzugt Präfixtreffer und begrenzt die Liste", () => {
    const r = searchEmoji(data, "smi", 5);
    expect(r.length).toBeLessThanOrEqual(5);
    expect(r[0]!.names.some((n) => n.startsWith("smi"))).toBe(true);
    expect(searchEmoji(data, "", 5)).toEqual([]);
  });
});
