-- 1000 (WP10): Dokumente. `trashed_with_parent` markiert Dokumente, die mit ihrem Elternobjekt in den
-- Papierkorb gewandert sind (docs/06 §1.10, Kaskade): nur diese werden beim Wiederherstellen des
-- Elternobjekts mit wiederhergestellt.

alter table document add column if not exists trashed_with_parent boolean not null default false;
