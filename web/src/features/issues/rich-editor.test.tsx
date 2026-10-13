import { render, screen, waitFor } from "@testing-library/react";
import * as Y from "yjs";
import { describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";

import { CollabProvider } from "@/features/collab/provider";

import { RichEditor } from "./rich-editor";

// ProseMirror misst Textbereiche; jsdom liefert dafür keine Geometrie.
Range.prototype.getClientRects ??= () =>
  ({
    length: 0,
    item: () => null,
    [Symbol.iterator]: [][Symbol.iterator],
  }) as unknown as DOMRectList;
Range.prototype.getBoundingClientRect ??= () => new DOMRect();

describe("RichEditor (TipTap)", () => {
  it("rendert Markdown als formatierten Text", async () => {
    render(<RichEditor value={"Hallo **Welt**"} ariaLabel="Description" onChange={vi.fn()} />);
    const box = await screen.findByRole("textbox", { name: "Description" });
    await waitFor(() => expect(box.querySelector("strong")).toHaveTextContent("Welt"));
    expect(box).toHaveAttribute("contenteditable", "true");
  });

  it("ist ohne editable nur zur Anzeige", async () => {
    render(<RichEditor value="Nur lesen" ariaLabel="Comment text" editable={false} />);
    const box = await screen.findByRole("textbox", { name: "Comment text" });
    expect(box).toHaveAttribute("contenteditable", "false");
    expect(box).toHaveTextContent("Nur lesen");
  });

  it("zeigt Tabellen, Aufgabenlisten und Bilder (Schema des Servers)", async () => {
    const md =
      "| a | b |\n| --- | --- |\n| 1 | 2 |\n\n- [x] erledigt\n- [ ] offen\n\n![Logo](https://acme.example/logo.png)";
    render(<RichEditor value={md} ariaLabel="Description" />);
    const box = await screen.findByRole("textbox", { name: "Description" });
    await waitFor(() => expect(box.querySelector("table")).not.toBeNull());
    expect(box.querySelectorAll("th, td")).toHaveLength(4);
    expect(box.querySelectorAll('ul[data-type="taskList"] li')).toHaveLength(2);
    expect(box.querySelector("img")?.getAttribute("src")).toBe("https://acme.example/logo.png");
  });

  it("verwirft beim Öffnen im Collab-Modus keine Server-Knoten (Tabelle, Aufgabe, Bild)", async () => {
    const provider = new CollabProvider({ url: "ws://x/collab/document/d1" });
    const frag = provider.doc.getXmlFragment("default");
    const el = (name: string, attrs: Record<string, string | boolean> = {}) => {
      const e = new Y.XmlElement(name);
      for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v as string);
      return e;
    };
    const para = (text: string) => {
      const p = el("paragraph");
      p.insert(0, [new Y.XmlText(text)]);
      return p;
    };
    const cell = (tag: string, text: string) => {
      const c = el(tag);
      c.insert(0, [para(text)]);
      return c;
    };
    const row = (tag: string, a: string, b: string) => {
      const r = el("tableRow");
      r.insert(0, [cell(tag, a), cell(tag, b)]);
      return r;
    };
    const table = el("table");
    table.insert(0, [row("tableHeader", "a", "b"), row("tableCell", "1", "2")]);
    const taskList = el("taskList");
    const item = el("taskItem", { checked: true as unknown as string });
    item.insert(0, [para("erledigt")]);
    taskList.insert(0, [item]);
    const image = el("image", { src: "https://acme.example/logo.png", alt: "Logo" });
    const imgPara = el("paragraph");
    imgPara.insert(0, [image]);
    frag.insert(0, [table, taskList, imgPara]);
    const before = frag.toString();

    render(
      <RichEditor
        value=""
        ariaLabel="Description"
        collab={{ provider, canWrite: true }}
      />,
    );
    const box = await screen.findByRole("textbox", { name: "Description" });
    await waitFor(() => expect(box.querySelector("table")).not.toBeNull());
    await new Promise((r) => setTimeout(r, 100));
    expect(box.querySelector('ul[data-type="taskList"] li')).not.toBeNull();
    expect(box.querySelector("img")).not.toBeNull();
    // Der gemeinsame Zustand bleibt unverändert (der Editor schreibt nichts zurück).
    expect(frag.toString()).toBe(before);
    provider.destroy();
  });
});
