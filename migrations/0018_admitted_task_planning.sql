-- Read-only model context and proposal records for an admitted general task.
CREATE TABLE task_model_contexts (
    id TEXT PRIMARY KEY,
    admission_id TEXT NOT NULL UNIQUE REFERENCES admission_revisions(id),
    snapshot_id TEXT NOT NULL REFERENCES verification_snapshots(id),
    context_json BLOB NOT NULL CHECK(length(context_json) <= 65536),
    stale_reason TEXT
);
CREATE TABLE task_model_proposals (
    context_id TEXT PRIMARY KEY REFERENCES task_model_contexts(id),
    request_id TEXT NOT NULL UNIQUE REFERENCES model_requests(id),
    proposal_json BLOB NOT NULL CHECK(length(proposal_json) <= 65536),
    stale_reason TEXT
);
CREATE TRIGGER immutable_task_model_context BEFORE UPDATE OF id, admission_id, snapshot_id, context_json ON task_model_contexts
BEGIN SELECT RAISE(ABORT, 'task model context is immutable'); END;
CREATE TRIGGER retain_task_model_context BEFORE DELETE ON task_model_contexts
BEGIN SELECT RAISE(ABORT, 'task model context cannot be deleted'); END;
CREATE TRIGGER monotonic_task_model_context_staleness BEFORE UPDATE OF stale_reason ON task_model_contexts
WHEN OLD.stale_reason IS NOT NULL
BEGIN SELECT RAISE(ABORT, 'stale task model context cannot become current again'); END;
CREATE TRIGGER immutable_task_model_proposal BEFORE UPDATE OF context_id, request_id, proposal_json ON task_model_proposals
BEGIN SELECT RAISE(ABORT, 'task model proposal is immutable'); END;
CREATE TRIGGER retain_task_model_proposal BEFORE DELETE ON task_model_proposals
BEGIN SELECT RAISE(ABORT, 'task model proposal cannot be deleted'); END;
CREATE TRIGGER monotonic_task_model_proposal_staleness BEFORE UPDATE OF stale_reason ON task_model_proposals
WHEN OLD.stale_reason IS NOT NULL
BEGIN SELECT RAISE(ABORT, 'stale task model proposal cannot become current again'); END;
