-- 3300 (WP36): Echtzeit-Mitbearbeitung (docs/06 §1.9 Stufe 2).
-- `document_content.content_state` existiert seit 0016 (Base64 eines Yjs-Updates V1); neu ist die
-- Epoche: sie kennzeichnet die CRDT-Linie. Wird der Zustand verworfen (Schreiben ohne Raum, Import),
-- entsteht beim nächsten Öffnen eine neue Epoche, und Clients mit altem lokalem Stand setzen sich
-- zurück, statt Inhalte zu verdoppeln.
alter table document_content add column content_epoch uuid;

-- Inline-Kommentar-Anker als relative Positionen (Base64 von Y.encodeRelativePosition), docs/06 §1.8.
alter table comment
  add column anchor_start text check (anchor_start is null or char_length(anchor_start) <= 512),
  add column anchor_end text check (anchor_end is null or char_length(anchor_end) <= 512),
  add constraint comment_anchor_pair check ((anchor_start is null) = (anchor_end is null)),
  add constraint comment_anchor_inline check (anchor_start is null or quoted_text is not null);
