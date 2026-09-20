-- One task/run keeps its accounting and recovery history across admissions.
CREATE TABLE admission_revisions (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    predecessor TEXT UNIQUE REFERENCES admission_revisions(id),
    request_key TEXT UNIQUE,
    reason TEXT NOT NULL,
    admission_json BLOB NOT NULL CHECK(length(admission_json) <= 65536),
    stale_reason TEXT
);
CREATE TABLE current_admission (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    admission_id TEXT NOT NULL REFERENCES admission_revisions(id)
);
CREATE TABLE admission_run_owners (
    admission_id TEXT PRIMARY KEY REFERENCES admission_revisions(id),
    run_id TEXT NOT NULL REFERENCES runs(id)
);
INSERT INTO admission_revisions(id, reason, admission_json)
SELECT json_extract(admission_json, '$.id'), 'Original admission', admission_json FROM task_admissions;
INSERT INTO current_admission(singleton, admission_id)
SELECT 1, id FROM admission_revisions;
INSERT INTO admission_run_owners(admission_id, run_id)
SELECT admission_id, run_id FROM task_admission_runs;
-- Keep legacy initialization and migration fixtures compatible.
CREATE TRIGGER register_initial_admission AFTER INSERT ON task_admissions
BEGIN
    INSERT INTO admission_revisions(id, reason, admission_json)
    VALUES (json_extract(NEW.admission_json, '$.id'), 'Original admission', NEW.admission_json);
    INSERT INTO current_admission(singleton, admission_id)
    VALUES (1, json_extract(NEW.admission_json, '$.id'));
END;
CREATE TRIGGER register_initial_admission_run AFTER INSERT ON task_admission_runs
BEGIN
    INSERT INTO admission_run_owners(admission_id, run_id) VALUES (NEW.admission_id, NEW.run_id);
END;
CREATE TRIGGER immutable_admission_revision BEFORE UPDATE OF sequence, id, predecessor, request_key, reason, admission_json ON admission_revisions
BEGIN SELECT RAISE(ABORT, 'admission revision is immutable'); END;
CREATE TRIGGER retain_admission_revision BEFORE DELETE ON admission_revisions
BEGIN SELECT RAISE(ABORT, 'admission history cannot be deleted'); END;
CREATE TRIGGER monotonic_admission_revision_staleness BEFORE UPDATE OF stale_reason ON admission_revisions
WHEN OLD.stale_reason IS NOT NULL
BEGIN SELECT RAISE(ABORT, 'stale admission cannot become current again'); END;
CREATE TRIGGER forward_current_admission BEFORE UPDATE ON current_admission
WHEN NOT EXISTS (
    SELECT 1 FROM admission_revisions WHERE id = NEW.admission_id
    AND predecessor = OLD.admission_id AND stale_reason IS NULL
) OR NOT EXISTS (
    SELECT 1 FROM admission_revisions WHERE id = OLD.admission_id AND stale_reason IS NOT NULL
)
BEGIN SELECT RAISE(ABORT, 'current admission must advance to a fresh successor'); END;
CREATE TRIGGER retain_current_admission BEFORE DELETE ON current_admission
BEGIN SELECT RAISE(ABORT, 'current admission cannot be deleted'); END;
CREATE TRIGGER immutable_admission_run_owner BEFORE UPDATE ON admission_run_owners
BEGIN SELECT RAISE(ABORT, 'admission run ownership is immutable'); END;
CREATE TRIGGER retain_admission_run_owner BEFORE DELETE ON admission_run_owners
BEGIN SELECT RAISE(ABORT, 'admission run ownership cannot be deleted'); END;
