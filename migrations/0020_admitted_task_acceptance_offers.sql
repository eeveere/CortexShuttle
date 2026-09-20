-- Review-only offer for an admitted task. User decision/finalization remains a
-- later phase and does not share this table.
CREATE TABLE task_acceptance_offers (
    id TEXT PRIMARY KEY,
    request_key TEXT NOT NULL UNIQUE,
    admission_id TEXT NOT NULL UNIQUE REFERENCES admission_revisions(id),
    offer_json BLOB NOT NULL CHECK(length(offer_json) <= 65536),
    stale_reason TEXT
);
CREATE TRIGGER immutable_task_acceptance_offer BEFORE UPDATE OF id, request_key, admission_id, offer_json ON task_acceptance_offers
BEGIN SELECT RAISE(ABORT, 'task acceptance offer is immutable'); END;
CREATE TRIGGER retain_task_acceptance_offer BEFORE DELETE ON task_acceptance_offers
BEGIN SELECT RAISE(ABORT, 'task acceptance offer cannot be deleted'); END;
CREATE TRIGGER monotonic_task_acceptance_offer_staleness BEFORE UPDATE OF stale_reason ON task_acceptance_offers
WHEN OLD.stale_reason IS NOT NULL
BEGIN SELECT RAISE(ABORT, 'stale task acceptance offer cannot become current again'); END;
