-- V2 read context is durable data, never a fixture action or a write grant.
CREATE TABLE admitted_edit_sessions (
    id TEXT PRIMARY KEY,
    permission_id TEXT NOT NULL UNIQUE REFERENCES task_write_permissions(id),
    definition_json BLOB NOT NULL CHECK(length(definition_json) <= 65536),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts BETWEEN 0 AND 5),
    read_count INTEGER NOT NULL DEFAULT 0 CHECK(read_count BETWEEN 0 AND 4),
    read_bytes INTEGER NOT NULL DEFAULT 0 CHECK(read_bytes BETWEEN 0 AND 6144),
    terminal_reason TEXT,
    action_id TEXT UNIQUE REFERENCES actions(id)
);
CREATE TABLE admitted_edit_turns (
    session_id TEXT NOT NULL REFERENCES admitted_edit_sessions(id),
    turn_index INTEGER NOT NULL CHECK(turn_index BETWEEN 0 AND 4),
    request_id TEXT NOT NULL UNIQUE REFERENCES model_requests(id),
    PRIMARY KEY(session_id, turn_index)
);
CREATE TABLE admitted_edit_observations (
    request_id TEXT PRIMARY KEY REFERENCES admitted_edit_turns(request_id),
    observation_json BLOB NOT NULL CHECK(length(observation_json) <= 16384),
    observation_hash TEXT NOT NULL,
    text_bytes INTEGER NOT NULL CHECK(text_bytes BETWEEN 0 AND 2048)
);
CREATE TRIGGER immutable_edit_session BEFORE UPDATE OF id, permission_id, definition_json ON admitted_edit_sessions
BEGIN SELECT RAISE(ABORT, 'edit session definition is immutable'); END;
CREATE TRIGGER retain_edit_session BEFORE DELETE ON admitted_edit_sessions
BEGIN SELECT RAISE(ABORT, 'edit session is retained'); END;
CREATE TRIGGER terminal_edit_session BEFORE UPDATE ON admitted_edit_sessions WHEN OLD.terminal_reason IS NOT NULL
BEGIN SELECT RAISE(ABORT, 'edit session is closed'); END;
CREATE TRIGGER monotonic_edit_counters BEFORE UPDATE ON admitted_edit_sessions
WHEN NEW.attempts < OLD.attempts OR NEW.read_count < OLD.read_count OR NEW.read_bytes < OLD.read_bytes
BEGIN SELECT RAISE(ABORT, 'edit budgets cannot be refunded'); END;
CREATE TRIGGER immutable_edit_turn BEFORE UPDATE ON admitted_edit_turns
BEGIN SELECT RAISE(ABORT, 'edit turn is immutable'); END;
CREATE TRIGGER retain_edit_turn BEFORE DELETE ON admitted_edit_turns
BEGIN SELECT RAISE(ABORT, 'edit turn is retained'); END;
CREATE TRIGGER immutable_edit_observation BEFORE UPDATE ON admitted_edit_observations
BEGIN SELECT RAISE(ABORT, 'edit observation is immutable'); END;
CREATE TRIGGER retain_edit_observation BEFORE DELETE ON admitted_edit_observations
BEGIN SELECT RAISE(ABORT, 'edit observation is retained'); END;
