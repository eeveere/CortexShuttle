CREATE TABLE evidence_qualifications (
    action_id TEXT PRIMARY KEY REFERENCES actions(id),
    bytes BLOB NOT NULL
);
CREATE TABLE consolidation_previews (
    request_key TEXT PRIMARY KEY,
    bytes BLOB NOT NULL
);
CREATE TABLE consolidation_results (
    request_key TEXT PRIMARY KEY,
    bytes BLOB NOT NULL
);
