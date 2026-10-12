# Vendored yrs 0.25.0 (MIT) mit zwei Korrektur-Patches

Quelle: crates.io `yrs` 0.25.0 (rustc 1.93 uebersetzt die neueren Versionen 0.27/0.28 wegen
`if let`-Guards nicht; 0.26 enthaelt Fehler 1 noch). Manifest: Dev-Abhaengigkeiten und Benches
entfernt. Der Fund stammt aus dem Zufallstest `fuzz_two_clients_converge_without_panic`
(lineal-app, collab::crdt) und `prepend_multiple*`.

1. `src/branch.rs`, `Branch::insert_at`: Einfuegen an Index 0 setzt `right` auf `self.start` (wie ab
   yrs 0.28). Ohne den Patch landet ein an Index 0 eingefuegter Knoten in nicht leeren Sequenzen
   abhaengig von der Client-ID hinter dem ersten Knoten.
2. `src/transaction.rs`, `apply_delete`: Die Schleife ueber die Eintraege einer Loeschspanne lief nur,
   wenn der erste Block der Spanne ein Eintrag war. Begann sie mit einem GC-Block (Gegenstelle hat
   bereits aufgeraeumt, lokal leben die Eintraege noch), blieben alle folgenden Eintraege
   ungeloescht: Server und Browser divergierten dauerhaft. Die Schleife laeuft jetzt immer.

Beide Stellen sind mit `LINEAL-PATCH` markiert. Entfernen, sobald auf eine yrs-Version mit beiden
Korrekturen aktualisiert werden kann (dann in `crates/lineal-app/Cargo.toml` wieder `yrs = "x"`
und `exclude` im Workspace streichen).
Lizenz: MIT (Autoren: Kevin Jahns, Bartosz Sypytkowski u. a.).
