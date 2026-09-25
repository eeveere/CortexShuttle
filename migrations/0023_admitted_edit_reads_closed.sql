-- S033 clarification (2026-09-24, Chunk 4a): a read whose smallest excerpt
-- would make the next request unrepresentable is refused, and further reads
-- are closed so the session keeps a patch turn. The session itself stays open.
ALTER TABLE admitted_edit_sessions ADD COLUMN reads_closed_reason TEXT
    CHECK(reads_closed_reason IS NULL OR length(reads_closed_reason) BETWEEN 1 AND 512);
CREATE TRIGGER final_edit_reads_closed BEFORE UPDATE OF reads_closed_reason ON admitted_edit_sessions
WHEN OLD.reads_closed_reason IS NOT NULL AND NEW.reads_closed_reason IS NOT OLD.reads_closed_reason
BEGIN SELECT RAISE(ABORT, 'closed edit reads cannot reopen'); END;
