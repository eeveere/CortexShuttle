CREATE TABLE task_admission_checks (
    admission_id TEXT NOT NULL,
    check_id TEXT NOT NULL,
    action_id TEXT NOT NULL UNIQUE,
    PRIMARY KEY(admission_id, check_id)
);

CREATE TRIGGER immutable_task_admission_check BEFORE UPDATE ON task_admission_checks
BEGIN SELECT RAISE(ABORT, 'task admission check binding is immutable'); END;

CREATE TRIGGER immutable_task_admission_check_delete BEFORE DELETE ON task_admission_checks
BEGIN SELECT RAISE(ABORT, 'task admission check binding cannot be deleted'); END;

INSERT INTO task_admission_checks(admission_id, check_id, action_id)
SELECT admission_id, check_id, action_id
FROM task_admission_runs
WHERE action_id IS NOT NULL;
