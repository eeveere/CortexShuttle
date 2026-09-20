use cortex_shuttle::{
    controller::ToolExecutor,
    fixture::Fixture,
    journal::{ActionIntent, ActionState, Grant, Journal, ToolCall},
    workspace::{TaskReader, create_demo, list_tasks, run_demo},
};
use tempfile::tempdir;

#[tokio::test]
async fn observer_coexists_with_controller_and_never_recovers_started_work() {
    let dir = tempdir().unwrap();
    let mut journal = Journal::open(&dir.path().join("journal.sqlite"))
        .await
        .unwrap();
    let fixture = Fixture::open_or_create(&dir.path().join("fixture")).unwrap();
    journal
        .ensure_run(fixture.root(), "workspace")
        .await
        .unwrap();
    let reader = TaskReader::open(dir.path()).await.unwrap();
    let before = reader.view().await.unwrap();
    journal
        .prepare(&ActionIntent {
            id: "one".into(),
            call: ToolCall::ReadFixture,
            input_hash: fixture.input_hash().unwrap(),
            grant: Grant {
                revision: 1,
                fixture_writes: false,
                process_authorization_hash: None,
            },
        })
        .await
        .unwrap();
    assert!(reader.view().await.unwrap().pending[0].contains("PREPARED"));
    journal.start("one").await.unwrap();
    assert!(reader.view().await.unwrap().pending[0].contains("STARTED"));
    assert_eq!(
        journal.action("one").await.unwrap().unwrap().state,
        ActionState::Started
    );
    assert_eq!(reader.view().await.unwrap().active_ms, before.active_ms);
    // Listing and CLI inspection also work while the owner lock is held.
    let cli = std::process::Command::new(env!("CARGO_BIN_EXE_shuttle"))
        .args(["task-view", "--json", "--state-dir"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(
        cli.status.success(),
        "{}",
        String::from_utf8_lossy(&cli.stderr)
    );
    assert!(String::from_utf8_lossy(&cli.stdout).contains("STARTED"));
    journal.close().await;
    assert!(reader.view().await.unwrap().pending[0].contains("STARTED"));
    let journal = Journal::open(&dir.path().join("journal.sqlite"))
        .await
        .unwrap();
    assert!(reader.view().await.unwrap().pending[0].contains("UNKNOWN"));
    journal.close().await;
    reader.close().await;
}

#[tokio::test]
async fn new_task_is_idle_until_explicit_shared_workflow_run_and_replay_is_stable() {
    let dir = tempdir().unwrap();
    let task = dir.path().join("new-task");
    create_demo(&task).await.unwrap();
    let reader = TaskReader::open(&task).await.unwrap();
    let before = reader.view().await.unwrap();
    assert!(before.scripted);
    assert!(before.actions.is_empty());
    assert_eq!(before.model_responses, 0);
    assert_eq!(
        std::fs::read(task.join("fixture/value.txt")).unwrap(),
        b"41\n"
    );
    assert_eq!(list_tasks(dir.path()).await.unwrap().len(), 1);
    assert!(create_demo(&task).await.is_err());
    assert!(run_demo(&task, Some("another-run")).await.is_err());
    run_demo(&task, Some(&before.run_id)).await.unwrap();
    let after = reader.view().await.unwrap();
    assert_eq!(after.phase, "awaiting_review");
    assert_eq!(after.actions.len(), 4);
    assert_eq!(after.model_responses, 5);
    assert_eq!(
        std::fs::read(task.join("fixture/value.txt")).unwrap(),
        b"42\n"
    );
    run_demo(&task, Some(&before.run_id)).await.unwrap();
    let reopened = reader.view().await.unwrap();
    assert_eq!(reopened.actions, after.actions);
    assert_eq!(reopened.model_responses, after.model_responses);
    assert!(reopened.acceptance_offer_id.is_none());
    reader.close().await;
}

#[tokio::test]
async fn unknown_task_cannot_start_and_missing_state_is_not_recreated() {
    let dir = tempdir().unwrap();
    assert!(TaskReader::open(&dir.path().join("absent")).await.is_err());
    assert!(!dir.path().join("absent").exists());
    let task = dir.path().join("unknown");
    create_demo(&task).await.unwrap();
    let mut journal = Journal::open(&task.join("journal.sqlite")).await.unwrap();
    let fixture = Fixture::open_or_create(&task.join("fixture")).unwrap();
    let id = journal.run().await.unwrap().unwrap().id;
    journal
        .prepare(&ActionIntent {
            id: "interrupted".into(),
            call: ToolCall::ReadFixture,
            input_hash: fixture.input_hash().unwrap(),
            grant: Grant {
                revision: 1,
                fixture_writes: false,
                process_authorization_hash: None,
            },
        })
        .await
        .unwrap();
    journal.start("interrupted").await.unwrap();
    journal.close().await;
    assert!(run_demo(&task, Some(&id)).await.is_err());
    let reader = TaskReader::open(&task).await.unwrap();
    let view = reader.view().await.unwrap();
    assert_eq!(view.model_responses, 0);
    assert!(view.pending[0].contains("UNKNOWN"));
    assert_eq!(
        std::fs::read(task.join("fixture/value.txt")).unwrap(),
        b"41\n"
    );
    reader.close().await;
}
