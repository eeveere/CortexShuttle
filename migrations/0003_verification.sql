CREATE TABLE verification_plans (
    revision TEXT PRIMARY KEY,
    plan_json TEXT NOT NULL
);
CREATE TABLE verification_snapshots (
    id TEXT PRIMARY KEY,
    bytes BLOB NOT NULL CHECK(length(bytes) <= 65536)
);
CREATE TABLE verification_actions (
    action_id TEXT PRIMARY KEY REFERENCES actions(id),
    plan_revision TEXT NOT NULL REFERENCES verification_plans(revision),
    check_id TEXT NOT NULL,
    pre_snapshot TEXT NOT NULL REFERENCES verification_snapshots(id)
);
CREATE TABLE verification_receipts (
    action_id TEXT PRIMARY KEY REFERENCES verification_actions(action_id),
    receipt_json TEXT NOT NULL,
    stale_reason TEXT
);
CREATE TRIGGER immutable_verification_plan BEFORE UPDATE ON verification_plans
BEGIN SELECT RAISE(ABORT, 'verification plan is immutable'); END;
CREATE TRIGGER immutable_verification_snapshot BEFORE UPDATE ON verification_snapshots
BEGIN SELECT RAISE(ABORT, 'verification snapshot is immutable'); END;
CREATE TRIGGER immutable_verification_binding BEFORE UPDATE ON verification_actions
BEGIN SELECT RAISE(ABORT, 'verification action binding is immutable'); END;
CREATE TRIGGER immutable_verification_receipt BEFORE UPDATE OF action_id, receipt_json ON verification_receipts
BEGIN SELECT RAISE(ABORT, 'verification receipt is immutable'); END;
CREATE TRIGGER monotonic_verification_staleness BEFORE UPDATE OF stale_reason ON verification_receipts
WHEN OLD.stale_reason IS NOT NULL
BEGIN SELECT RAISE(ABORT, 'stale evidence cannot become current again'); END;
