ALTER TABLE runs ADD COLUMN active_ms INTEGER NOT NULL DEFAULT 0 CHECK(active_ms >= 0);
ALTER TABLE runs ADD COLUMN active_limit_ms INTEGER NOT NULL DEFAULT 3600000 CHECK(active_limit_ms > 0);
ALTER TABLE runs ADD COLUMN no_progress_ms INTEGER NOT NULL DEFAULT 0 CHECK(no_progress_ms >= 0);
ALTER TABLE runs ADD COLUMN no_progress_actions INTEGER NOT NULL DEFAULT 0 CHECK(no_progress_actions >= 0);
ALTER TABLE runs ADD COLUMN progress_epoch INTEGER NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN stall_reason TEXT;
ALTER TABLE runs ADD COLUMN replans INTEGER NOT NULL DEFAULT 0 CHECK(replans BETWEEN 0 AND 1);
CREATE TABLE active_spans (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    reserved_ms INTEGER NOT NULL CHECK(reserved_ms > 0),
    elapsed_ms INTEGER CHECK(elapsed_ms >= 0),
    state TEXT NOT NULL CHECK(state IN ('started', 'observed', 'unknown'))
);
CREATE UNIQUE INDEX one_active_span ON active_spans((1)) WHERE state = 'started';
CREATE TRIGGER immutable_span_intent BEFORE UPDATE OF id, kind, reserved_ms ON active_spans
BEGIN SELECT RAISE(ABORT, 'active span intent is immutable'); END;
CREATE TRIGGER immutable_span_result BEFORE UPDATE ON active_spans WHEN OLD.state != 'started'
BEGIN SELECT RAISE(ABORT, 'active span result is immutable'); END;
CREATE TABLE model_requests (
    id TEXT PRIMARY KEY,
    ordinal INTEGER NOT NULL UNIQUE,
    intent_json TEXT NOT NULL CHECK(length(CAST(intent_json AS BLOB)) <= 65536),
    state TEXT NOT NULL CHECK(state IN ('prepared', 'started', 'succeeded', 'failed', 'unknown')),
    result_json TEXT CHECK(length(CAST(result_json AS BLOB)) <= 65536),
    application TEXT CHECK(length(CAST(application AS BLOB)) <= 4096),
    applied INTEGER NOT NULL DEFAULT 0 CHECK(applied IN (0, 1))
);
CREATE UNIQUE INDEX one_pending_model_request ON model_requests((1)) WHERE applied = 0;
CREATE TRIGGER immutable_model_intent BEFORE UPDATE OF id, ordinal, intent_json ON model_requests
BEGIN SELECT RAISE(ABORT, 'model request intent is immutable'); END;
CREATE TRIGGER immutable_model_result BEFORE UPDATE OF state, result_json ON model_requests WHEN OLD.result_json IS NOT NULL OR OLD.state = 'unknown'
BEGIN SELECT RAISE(ABORT, 'model request result is immutable'); END;
CREATE TRIGGER monotonic_model_application BEFORE UPDATE OF applied ON model_requests WHEN OLD.applied = 1
BEGIN SELECT RAISE(ABORT, 'model request already applied'); END;
CREATE TRIGGER immutable_model_application BEFORE UPDATE OF application ON model_requests WHEN OLD.application IS NOT NULL
BEGIN SELECT RAISE(ABORT, 'model application is immutable'); END;
CREATE TABLE progress_evidence (fingerprint TEXT PRIMARY KEY);
CREATE TABLE stall_resumptions (
    id TEXT PRIMARY KEY,
    reason TEXT NOT NULL CHECK(length(CAST(reason AS BLOB)) BETWEEN 1 AND 4096)
);
CREATE TRIGGER immutable_stall_resumption BEFORE UPDATE ON stall_resumptions
BEGIN SELECT RAISE(ABORT, 'stall resumption is immutable'); END;

-- Older runs have only a response counter and process-time ledger. Preserve that
-- lower bound and explicitly label unavailable historical request observations.
UPDATE runs SET active_ms = process_active_ms;
WITH RECURSIVE legacy(n) AS (
    SELECT 0 WHERE EXISTS(SELECT 1 FROM runs WHERE model_responses > 0)
    UNION ALL SELECT n + 1 FROM legacy WHERE n + 1 < (SELECT model_responses FROM runs)
)
INSERT INTO model_requests(id, ordinal, intent_json, state, result_json, applied)
SELECT runs.id || '/request/' || legacy.n, legacy.n,
    '{"version":1,"provider":"legacy_budget_only","purpose":"reservation","context":{"actions":[],"input_hash":"","replan_direction":null},"grant":{"revision":0,"fixture_writes":false},"timeout_ms":0}',
    'failed',
    '{"reply":null,"error":"Legacy budget reservation; no provider observation available","elapsed_ms":0,"limitation":"Historical provider identity, response, usage and elapsed time were not recorded; this is an accounting placeholder."}',
    1 FROM legacy CROSS JOIN runs;
