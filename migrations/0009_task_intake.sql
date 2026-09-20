CREATE TABLE task_intakes (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    intake_json BLOB NOT NULL
);

CREATE TRIGGER immutable_task_intake BEFORE UPDATE ON task_intakes
BEGIN SELECT RAISE(ABORT, 'task intake is immutable'); END;

CREATE TRIGGER immutable_task_intake_delete BEFORE DELETE ON task_intakes
BEGIN SELECT RAISE(ABORT, 'task intake cannot be deleted'); END;
