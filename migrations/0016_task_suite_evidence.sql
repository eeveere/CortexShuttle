CREATE TABLE task_suite_evidence (
    admission_id TEXT PRIMARY KEY,
    evidence_json TEXT NOT NULL,
    stale_reason TEXT
);

CREATE TRIGGER immutable_task_suite_evidence BEFORE UPDATE OF admission_id, evidence_json ON task_suite_evidence
BEGIN SELECT RAISE(ABORT, 'task suite evidence is immutable'); END;

CREATE TRIGGER monotonic_task_suite_evidence_staleness BEFORE UPDATE OF stale_reason ON task_suite_evidence
WHEN OLD.stale_reason IS NOT NULL
BEGIN SELECT RAISE(ABORT, 'stale task suite evidence cannot become current again'); END;
