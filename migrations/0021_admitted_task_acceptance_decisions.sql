-- Explicit, terminal user choices for review-only admitted-task offers.
CREATE TABLE task_acceptance_decisions (
    request_key TEXT PRIMARY KEY,
    offer_id TEXT NOT NULL UNIQUE REFERENCES task_acceptance_offers(id),
    choice TEXT NOT NULL CHECK(choice IN ('accept', 'reject')),
    decision_json BLOB NOT NULL CHECK(length(decision_json) <= 65536)
);
CREATE UNIQUE INDEX one_task_run_acceptance ON task_acceptance_decisions((1)) WHERE choice = 'accept';
CREATE TRIGGER immutable_task_acceptance_decision BEFORE UPDATE ON task_acceptance_decisions
BEGIN SELECT RAISE(ABORT, 'task acceptance decision is immutable'); END;
CREATE TRIGGER retain_task_acceptance_decision BEFORE DELETE ON task_acceptance_decisions
BEGIN SELECT RAISE(ABORT, 'task acceptance decision cannot be deleted'); END;
