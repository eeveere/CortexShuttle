ALTER TABLE task_admission_runs ADD COLUMN action_id TEXT;
CREATE UNIQUE INDEX task_admission_action_id ON task_admission_runs(action_id) WHERE action_id IS NOT NULL;
