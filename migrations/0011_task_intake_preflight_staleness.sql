ALTER TABLE task_intake_preflights ADD COLUMN stale_reason TEXT;

DROP TRIGGER immutable_task_intake_preflight;
CREATE TRIGGER immutable_task_intake_preflight BEFORE UPDATE OF intake_id, plan_revision, snapshot_id ON task_intake_preflights
BEGIN SELECT RAISE(ABORT, 'task intake preflight is immutable'); END;

CREATE TRIGGER monotonic_task_intake_preflight_staleness
BEFORE UPDATE OF stale_reason ON task_intake_preflights
WHEN OLD.stale_reason IS NOT NULL
BEGIN SELECT RAISE(ABORT, 'stale intake preflight cannot become current again'); END;
