CREATE TABLE task_intake_preflights (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    intake_id TEXT NOT NULL,
    plan_revision TEXT NOT NULL REFERENCES verification_plans(revision),
    snapshot_id TEXT NOT NULL REFERENCES verification_snapshots(id),
    UNIQUE(intake_id, snapshot_id)
);

CREATE TRIGGER immutable_task_intake_preflight BEFORE UPDATE OF intake_id, plan_revision, snapshot_id ON task_intake_preflights
BEGIN SELECT RAISE(ABORT, 'task intake preflight is immutable'); END;

CREATE TRIGGER immutable_task_intake_preflight_delete BEFORE DELETE ON task_intake_preflights
BEGIN SELECT RAISE(ABORT, 'task intake preflight cannot be deleted'); END;
