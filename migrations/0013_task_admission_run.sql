CREATE TABLE task_admission_runs (
    admission_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL UNIQUE REFERENCES runs(id),
    check_id TEXT NOT NULL
);

CREATE TRIGGER immutable_task_admission_run BEFORE UPDATE ON task_admission_runs
BEGIN SELECT RAISE(ABORT, 'task admission run binding is immutable'); END;

CREATE TRIGGER immutable_task_admission_run_delete BEFORE DELETE ON task_admission_runs
BEGIN SELECT RAISE(ABORT, 'task admission run binding cannot be deleted'); END;
