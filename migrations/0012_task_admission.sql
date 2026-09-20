CREATE TABLE task_admissions (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    admission_json BLOB NOT NULL
);

CREATE TRIGGER immutable_task_admission BEFORE UPDATE ON task_admissions
BEGIN SELECT RAISE(ABORT, 'task admission is immutable'); END;

CREATE TRIGGER immutable_task_admission_delete BEFORE DELETE ON task_admissions
BEGIN SELECT RAISE(ABORT, 'task admission cannot be deleted'); END;
