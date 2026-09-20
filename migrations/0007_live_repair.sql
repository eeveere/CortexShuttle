CREATE TABLE live_repairs (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    configuration BLOB NOT NULL,
    phase TEXT NOT NULL CHECK(phase IN ('baseline', 'model', 'final'))
);
