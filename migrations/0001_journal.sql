CREATE TABLE runs (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    id TEXT NOT NULL UNIQUE,
    workspace_root TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    objective TEXT NOT NULL,
    phase TEXT NOT NULL CHECK(phase IN ('ready', 'executing', 'delivery_pending', 'paused', 'awaiting_review')),
    reason TEXT NOT NULL,
    model_responses INTEGER NOT NULL DEFAULT 0 CHECK(model_responses >= 0 AND model_responses <= 64)
);
CREATE TABLE transitions (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    phase TEXT NOT NULL,
    reason TEXT NOT NULL
);
CREATE TABLE artifacts (
    hash TEXT PRIMARY KEY,
    bytes BLOB NOT NULL
);
CREATE TABLE actions (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    intent_json TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('prepared', 'started', 'succeeded', 'failed', 'cancelled', 'unknown')),
    result_json TEXT
);
CREATE UNIQUE INDEX one_inflight_action ON actions((1)) WHERE state IN ('prepared', 'started');
CREATE TABLE deliveries (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    request_key TEXT NOT NULL UNIQUE,
    action_id TEXT REFERENCES actions(id),
    request_json TEXT NOT NULL,
    receipt_json TEXT
);
CREATE TRIGGER immutable_action_intent BEFORE UPDATE OF id, intent_json ON actions
BEGIN SELECT RAISE(ABORT, 'action intent is immutable'); END;
CREATE TRIGGER immutable_action_result BEFORE UPDATE ON actions WHEN OLD.result_json IS NOT NULL
BEGIN SELECT RAISE(ABORT, 'observed action is immutable'); END;
CREATE TRIGGER immutable_delivery_request BEFORE UPDATE OF request_key, action_id, request_json ON deliveries
BEGIN SELECT RAISE(ABORT, 'delivery request is immutable'); END;
CREATE TRIGGER immutable_delivery_receipt BEFORE UPDATE OF receipt_json ON deliveries WHEN OLD.receipt_json IS NOT NULL
BEGIN SELECT RAISE(ABORT, 'delivery receipt is immutable'); END;
CREATE TRIGGER immutable_artifact BEFORE UPDATE ON artifacts
BEGIN SELECT RAISE(ABORT, 'artifact is immutable'); END;
