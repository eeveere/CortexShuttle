CREATE TABLE task_workflow (
    run_id TEXT PRIMARY KEY REFERENCES runs(id),
    kind TEXT NOT NULL CHECK(kind = 'scripted_fixture_v1')
);
CREATE TRIGGER immutable_task_workflow BEFORE UPDATE ON task_workflow
BEGIN SELECT RAISE(ABORT, 'task workflow is immutable'); END;
