//! The edit review reads real journals from before migrations 0022 and 0023
//! through the read-only `TaskReader`, without migrating them.
use cortex_shuttle::{journal::Journal, workspace::TaskReader};
use sqlx::{Connection, SqliteConnection};
use tempfile::tempdir;

async fn strip(path: &std::path::Path, statements: &[&str]) {
    let mut conn = SqliteConnection::connect(&format!("sqlite://{}", path.display()))
        .await
        .unwrap();
    for statement in statements {
        sqlx::query(statement).execute(&mut conn).await.unwrap();
    }
    conn.close().await.unwrap();
}

#[tokio::test]
async fn real_journals_older_than_the_edit_session_migrations_still_review() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("journal.sqlite");
    Journal::open(&path).await.unwrap().close().await;

    // Current schema, no session yet: nothing to review.
    let reader = TaskReader::open(dir.path()).await.unwrap();
    assert!(reader.edit_review().await.unwrap().is_none());
    reader.close().await;

    // Migration 0022 without 0023: the reads-closed column does not exist.
    strip(
        &path,
        &[
            "DROP TRIGGER final_edit_reads_closed",
            "ALTER TABLE admitted_edit_sessions DROP COLUMN reads_closed_reason",
        ],
    )
    .await;
    let reader = TaskReader::open(dir.path()).await.unwrap();
    assert!(reader.edit_review().await.unwrap().is_none());
    reader.close().await;

    // Neither migration: the session tables do not exist at all.
    strip(
        &path,
        &[
            "DROP TABLE admitted_edit_turns",
            "DROP TABLE admitted_edit_sessions",
        ],
    )
    .await;
    let reader = TaskReader::open(dir.path()).await.unwrap();
    assert!(reader.edit_review().await.unwrap().is_none());
    reader.close().await;

    // Reading did not migrate the journal back: the tables are still absent.
    let mut conn = SqliteConnection::connect(&format!("sqlite://{}", path.display()))
        .await
        .unwrap();
    let tables: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_schema WHERE name IN ('admitted_edit_sessions', 'admitted_edit_turns')",
    )
    .fetch_one(&mut conn)
    .await
    .unwrap();
    assert_eq!(tables, 0);
}
