-- Explicit human write permissions for one saved admitted-task proposal.
-- This migration grants no writer or process capability by itself.
CREATE TABLE task_write_permissions (
    id TEXT PRIMARY KEY,
    request_key TEXT NOT NULL UNIQUE,
    admission_id TEXT NOT NULL REFERENCES admission_revisions(id),
    context_id TEXT NOT NULL UNIQUE REFERENCES task_model_contexts(id),
    snapshot_id TEXT NOT NULL REFERENCES verification_snapshots(id),
    proposal_request_id TEXT NOT NULL UNIQUE REFERENCES model_requests(id),
    actor TEXT NOT NULL,
    granted_unix_ms INTEGER NOT NULL,
    allowed_paths_json BLOB NOT NULL CHECK(length(allowed_paths_json) <= 65536),
    stale_reason TEXT
);
CREATE TABLE task_write_permission_revocations (
    permission_id TEXT PRIMARY KEY REFERENCES task_write_permissions(id),
    request_key TEXT NOT NULL UNIQUE,
    actor TEXT NOT NULL,
    reason TEXT NOT NULL,
    revoked_unix_ms INTEGER NOT NULL
);
CREATE TRIGGER immutable_task_write_permission BEFORE UPDATE OF id, request_key, admission_id, context_id, snapshot_id, proposal_request_id, actor, granted_unix_ms, allowed_paths_json ON task_write_permissions
BEGIN SELECT RAISE(ABORT, 'task write permission is immutable'); END;
CREATE TRIGGER retain_task_write_permission BEFORE DELETE ON task_write_permissions
BEGIN SELECT RAISE(ABORT, 'task write permission cannot be deleted'); END;
CREATE TRIGGER monotonic_task_write_permission_staleness BEFORE UPDATE OF stale_reason ON task_write_permissions
WHEN OLD.stale_reason IS NOT NULL
BEGIN SELECT RAISE(ABORT, 'stale task write permission cannot become current again'); END;
CREATE TRIGGER immutable_task_write_permission_revocation BEFORE UPDATE ON task_write_permission_revocations
BEGIN SELECT RAISE(ABORT, 'task write permission revocation is immutable'); END;
CREATE TRIGGER retain_task_write_permission_revocation BEFORE DELETE ON task_write_permission_revocations
BEGIN SELECT RAISE(ABORT, 'task write permission revocation cannot be deleted'); END;
