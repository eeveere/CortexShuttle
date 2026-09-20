ALTER TABLE runs ADD COLUMN process_active_ms INTEGER NOT NULL DEFAULT 0 CHECK(process_active_ms >= 0);
ALTER TABLE actions ADD COLUMN reserved_process_ms INTEGER NOT NULL DEFAULT 0 CHECK(reserved_process_ms >= 0);
