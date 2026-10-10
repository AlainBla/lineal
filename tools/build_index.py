"""Baut docs/index.html aus allen Kapiteln (Markdown -> HTML, Mermaid clientseitig).

Aufruf: python3 tools/build_index.py
Benötigt: Python-Paket `markdown`.
"""
import html
import os
import re

import markdown

DOCS = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "docs")
CHAPTERS = [
    ("README.md", "Übersicht"),
    ("00-konventionen.md", "00 Konventionen"),
    ("01-domaenenmodell.md", "01 Domänenmodell"),
    ("02-teams-rechte.md", "02 Teams & Rechte"),
    ("03-issues-workflow.md", "03 Issues & Workflow"),
    ("04-planung.md", "04 Planung"),
    ("05-views-suche-inbox.md", "05 Views, Suche, Inbox"),
    ("06-dokumente-templates.md", "06 Dokumente & Templates"),
    ("07-mcp-server.md", "07 MCP-Server"),
    ("08-userflows.md", "08 User Flows"),
    ("09-api-events.md", "09 API & Events"),
    ("10-architektur-umsetzung.md", "10 Architektur & Plan"),
    ("11-implementierungsarchitektur.md", "11 Implementierungsarchitektur"),
    ("12-arbeitspakete.md", "12 Arbeitspakete"),
    ("13-import-linear.md", "13 Import aus Linear"),
    ("14-firmenweite-sicht.md", "14 Firmenweite Sicht"),
]
MERMAID = re.compile(r"```mermaid\n(.*?)```", re.S)


def chapter_id(fname):
    return "ch-" + fname[:-3].lower()


def render(fname):
    src = open(os.path.join(DOCS, fname), encoding="utf-8").read()
    blocks = []

    def stash(m):
        blocks.append(m.group(1))
        return f"\n\nMERMAIDBLOCK{len(blocks) - 1}\n\n"

    src = MERMAID.sub(stash, src)
    md = markdown.Markdown(extensions=["tables", "fenced_code", "toc", "sane_lists"],
                           extension_configs={"toc": {"slugify": lambda v, s: chapter_id(fname) + "-" + re.sub(r"[^a-z0-9]+", "-", v.lower()).strip("-")}})
    out = md.convert(src)
    for i, b in enumerate(blocks):
        out = out.replace(f"<p>MERMAIDBLOCK{i}</p>", f'<pre class="mermaid">{html.escape(b)}</pre>')
    # Links zwischen Kapiteln auf Abschnitte dieser Seite umbiegen
    for f, _ in CHAPTERS:
        out = out.replace(f'href="{f}"', f'href="#{chapter_id(f)}"')
        out = re.sub(rf'href="{re.escape(f)}#[^"]*"', f'href="#{chapter_id(f)}"', out)
    return out, len(blocks)


def main():
    nav, body, diag = [], [], []
    for f, title in CHAPTERS:
        if not os.path.exists(os.path.join(DOCS, f)):
            continue
        content, n = render(f)
        cid = chapter_id(f)
        nav.append(f'<a href="#{cid}">{html.escape(title)}</a>')
        body.append(f'<section id="{cid}" class="chapter"><div class="src">Quelle: <a href="{f}">{f}</a></div>{content}</section>')
        diag.append(f"<li><a href=\"#{cid}\">{html.escape(title)}</a>: {n} Diagramme</li>")
    page = TEMPLATE.replace("{{NAV}}", "\n".join(nav)).replace("{{BODY}}", "\n".join(body)).replace("{{DIAG}}", "\n".join(diag))
    open(os.path.join(DOCS, "index.html"), "w", encoding="utf-8").write(page)
    print("index.html geschrieben")


TEMPLATE = """<!doctype html>
<html lang="de"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Lineal – Implementationsdokumentation</title>
<style>
:root{--bg:#fbfbfc;--fg:#1f2328;--muted:#6b6f76;--accent:#5e6ad2;--line:#e4e5e9;--code:#f3f4f6}
*{box-sizing:border-box}body{margin:0;font:15px/1.6 system-ui,-apple-system,Segoe UI,Roboto,sans-serif;color:var(--fg);background:var(--bg)}
nav{position:fixed;top:0;left:0;bottom:0;width:250px;overflow:auto;padding:20px 14px;border-right:1px solid var(--line);background:#fff}
nav h1{font-size:16px;margin:0 0 12px}nav a{display:block;padding:5px 8px;border-radius:6px;color:var(--fg);text-decoration:none;font-size:14px}
nav a:hover{background:var(--code)}main{margin-left:250px;padding:24px 48px;max-width:1150px}
.chapter{border-bottom:2px solid var(--line);padding-bottom:40px;margin-bottom:40px}
.src{font-size:12px;color:var(--muted)}h1,h2,h3{line-height:1.25}h2{border-bottom:1px solid var(--line);padding-bottom:4px;margin-top:2em}
a{color:var(--accent)}table{border-collapse:collapse;margin:12px 0;display:block;overflow:auto;font-size:14px}
th,td{border:1px solid var(--line);padding:5px 9px;vertical-align:top}th{background:var(--code)}
code{background:var(--code);padding:1px 4px;border-radius:4px;font-size:13px}pre{background:var(--code);padding:12px;border-radius:8px;overflow:auto}
pre code{background:none;padding:0}pre.mermaid{background:#fff;border:1px solid var(--line);text-align:center}
#diagramme ul{columns:2}
@media(max-width:900px){nav{position:static;width:auto;border:0}main{margin:0;padding:16px}}
</style></head><body>
<nav><h1>Lineal-Doku</h1><a href="#diagramme">Diagramm-Übersicht</a>
{{NAV}}
<a href="mcp-tools.snapshot.json">MCP-Tool-Snapshot (JSON)</a></nav>
<main>
<section id="diagramme" class="chapter"><h1>Diagramm-Übersicht</h1>
<p>Alle Diagramme sind Mermaid und werden unten im jeweiligen Kapitel gerendert. Die Kapitel-Markdown-Dateien sind die maßgebliche Quelle; diese Seite wird mit <code>python3 tools/build_index.py</code> neu erzeugt.</p>
<ul>{{DIAG}}</ul></section>
{{BODY}}
</main>
<script src="assets/mermaid.min.js"></script>
<script>mermaid.initialize({startOnLoad:true,securityLevel:'strict',theme:'neutral',er:{useMaxWidth:true},flowchart:{useMaxWidth:true}});</script>
</body></html>
"""

if __name__ == "__main__":
    main()
