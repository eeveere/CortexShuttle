CREATE TABLE runs_next (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    id TEXT NOT NULL UNIQUE,
    workspace_root TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    objective TEXT NOT NULL,
    phase TEXT NOT NULL CHECK(phase IN ('ready', 'executing', 'delivery_pending', 'paused', 'awaiting_review', 'awaiting_acceptance', 'finalizing', 'finalized')),
    reason TEXT NOT NULL,
    model_responses INTEGER NOT NULL DEFAULT 0 CHECK(model_responses >= 0 AND model_responses <= 64),
    process_active_ms INTEGER NOT NULL DEFAULT 0 CHECK(process_active_ms >= 0)
);
INSERT INTO runs_next SELECT * FROM runs;
DROP TABLE runs;
ALTER TABLE runs_next RENAME TO runs;

CREATE TABLE acceptance_offers (
    id TEXT PRIMARY KEY,
    offer_json TEXT NOT NULL CHECK(length(CAST(offer_json AS BLOB)) <= 65536),
    stale_reason TEXT
);
CREATE TABLE acceptance_decisions (
    request_key TEXT PRIMARY KEY,
    offer_id TEXT NOT NULL UNIQUE REFERENCES acceptance_offers(id),
    choice TEXT NOT NULL CHECK(choice IN ('accept', 'reject')),
    decision_json TEXT NOT NULL CHECK(length(CAST(decision_json AS BLOB)) <= 65536)
);
ALTER TABLE runs ADD COLUMN active_acceptance_offer_id TEXT REFERENCES acceptance_offers(id);
CREATE UNIQUE INDEX one_run_acceptance ON acceptance_decisions((1)) WHERE choice = 'accept';
CREATE TRIGGER immutable_acceptance_offer BEFORE UPDATE OF id, offer_json ON acceptance_offers
BEGIN SELECT RAISE(ABORT, 'acceptance offer is immutable'); END;
CREATE TRIGGER monotonic_offer_staleness BEFORE UPDATE OF stale_reason ON acceptance_offers WHEN OLD.stale_reason IS NOT NULL
BEGIN SELECT RAISE(ABORT, 'stale offer cannot become current again'); END;
CREATE TRIGGER immutable_acceptance_decision BEFORE UPDATE ON acceptance_decisions
BEGIN SELECT RAISE(ABORT, 'user decision is immutable'); END;
