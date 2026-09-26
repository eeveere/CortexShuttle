use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use cortex_shuttle::{
    acceptance::{UserChoice, UserResponse},
    adapter::CortexWeaveAdapter,
    controller::{Controller, Fault, ToolExecutor, flush_outbox},
    edit_review,
    fixture::Fixture,
    journal::{ActionIntent, Grant, Journal, ToolCall},
    llama::{LlamaModel, LlamaProfile},
    process::{Cancellation, ProcessExecutor, ProcessLimits, ProcessSpec, hash_executable},
    ui::{self, PreviewState, TaskOperation},
    verification::{ReceiptView, SourceSnapshot, SuiteEvidenceView, VerificationPlan},
    workspace::{self, TaskAdmission, TaskReader},
};
use cortexweave::{AppConfig, CortexWeaveService};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Serialize)]
struct AdmittedCheckStatus {
    check_id: String,
    name: String,
    status: String,
    action_id: Option<String>,
    waiver_reason: Option<String>,
    receipt: Option<ReceiptView>,
}

#[derive(Serialize)]
struct AdmittedSuiteStatus {
    checks: Vec<AdmittedCheckStatus>,
    evidence: Option<SuiteEvidenceView>,
}

#[derive(Parser)]
#[command(name = "shuttle", version, about = "Shuttle development foundation")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run an authorized isolated Qwen repair with real baseline/final unittest checks.
    LiveRepair {
        #[arg(long)]
        profile: PathBuf,
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        python: PathBuf,
        #[arg(long, required = true)]
        approve_fixture_writes: bool,
        #[arg(long, required = true)]
        approve_host_execution: bool,
    },
    /// Qualify an existing snapshot-bound observation and deliver its factual evidence.
    QualifyEvidence {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        action_id: String,
        #[arg(long, value_parser = ["rustc", "unittest"])]
        producer: String,
    },
    /// Capture a fixed local worker profile without loading or reconfiguring it.
    LlamaProfile {
        #[arg(long, default_value = "http://127.0.0.1:8080")]
        endpoint: String,
        #[arg(long)]
        model: String,
        #[arg(long)]
        server_executable: PathBuf,
        #[arg(long)]
        model_file: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        thinking: bool,
    },
    /// Run one durable local-model step in an isolated development fixture.
    LlamaStep {
        #[arg(long)]
        profile: PathBuf,
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        approve_fixture_writes: bool,
    },
    /// Inspect durable run budgets and provider attempt summaries.
    RunStatus {
        #[arg(long)]
        state_dir: PathBuf,
    },
    /// Record caller direction for the single permitted stall resumption.
    ResumeStall {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        request_key: String,
        #[arg(long)]
        reason: String,
    },
    /// Run a named check from an explicitly approved verification plan.
    Verify {
        #[arg(long)]
        workspace: PathBuf,
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        check_id: String,
        #[arg(long)]
        action_id: String,
        #[arg(long, required = true)]
        approve_host_execution: bool,
    },
    /// Offer an exact verified plan for explicit user review.
    AcceptanceOffer {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        plan_revision: String,
    },
    /// Show the complete saved acceptance offer, including waivers and limitations.
    AcceptanceReview {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        offer_id: String,
    },
    /// Respond to a saved offer from an interactive terminal; no automatic approval.
    AcceptanceRespond {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        offer_id: String,
        #[arg(long, value_enum)]
        choice: UserChoice,
    },
    /// Resume the ordered native finalization of a previously accepted run.
    Finalize {
        #[arg(long)]
        state_dir: PathBuf,
    },
    /// Print a command specification for inspection before granting host execution.
    ProcessSpec {
        #[arg(long)]
        executable: PathBuf,
        #[arg(long, default_value_t = 120_000)]
        timeout_ms: u64,
        #[arg(last = true)]
        arguments: Vec<String>,
    },
    /// Execute or recover one explicitly approved host command (Windows 10+ or Linux).
    Process {
        #[arg(long)]
        workspace: PathBuf,
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        spec: PathBuf,
        #[arg(long, required = true)]
        input: Vec<PathBuf>,
        /// Authorize the exact specification as host code execution.
        #[arg(long, required = true)]
        approve_host_execution: bool,
    },
    /// Run or resume the isolated scripted recovery fixture (no inference server required).
    Demo {
        #[arg(long, default_value = ".shuttle/demo")]
        state_dir: PathBuf,
    },
    /// Open the interaction prototype; Tab cycles sample states.
    Preview {
        #[arg(long, value_enum, default_value = "idle")]
        state: PreviewState,
        #[arg(long)]
        reduced_color: bool,
        #[arg(long)]
        export: Option<PathBuf>,
        #[arg(long, default_value_t = 120)]
        width: u16,
        #[arg(long, default_value_t = 36)]
        height: u16,
    },
    /// Display one persisted run without executing, granting, accepting or finalizing work.
    TaskView {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        reduced_color: bool,
        /// Print the same durable projection for headless callers.
        #[arg(long)]
        json: bool,
    },
    /// Choose or open a task; registered scripted fixtures require explicit start confirmation.
    Task {
        #[arg(long)]
        state_dir: Option<PathBuf>,
        #[arg(long, default_value = ".shuttle")]
        root: PathBuf,
        #[arg(long)]
        reduced_color: bool,
        /// Saved local-model profile used only for the explicit plan/edit operations.
        #[arg(long)]
        profile: Option<PathBuf>,
        /// Authorize the saved verification commands when Ctrl+V is explicitly confirmed.
        #[arg(long)]
        approve_host_execution: bool,
        /// Create this general task intake before opening the workspace.
        #[arg(long)]
        workspace: Option<PathBuf>,
        #[arg(long)]
        objective: Option<String>,
        #[arg(long = "constraint")]
        constraints: Vec<String>,
        #[arg(long)]
        plan: Option<PathBuf>,
    },
    /// Create a new isolated scripted task, without running its tools.
    TaskNew {
        #[arg(long, default_value = ".shuttle")]
        root: PathBuf,
    },
    /// Save an immutable general-task intake; this does not admit a workflow or run commands.
    TaskIntake {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        workspace: PathBuf,
        #[arg(long)]
        objective: String,
        #[arg(long = "constraint")]
        constraints: Vec<String>,
        #[arg(long)]
        plan: PathBuf,
    },
    /// Save a fresh source snapshot for a general-task intake without admitting work.
    TaskPreflight {
        #[arg(long)]
        state_dir: PathBuf,
    },
    /// Save the general-verification workflow contract after a matching preflight.
    TaskAdmit {
        #[arg(long)]
        state_dir: PathBuf,
    },
    /// Ask the local model for a bounded, read-only plan from an admitted task context.
    TaskPlan {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        profile: PathBuf,
    },
    /// Show the current saved proposal and its write-permission state without changing it.
    TaskWriteStatus {
        #[arg(long)]
        state_dir: PathBuf,
    },
    /// Show what the admitted edit changed (or why it was blocked): the exact
    /// compact hunks, hashes and session ledger, from saved state only.
    TaskEditReview {
        #[arg(long)]
        state_dir: PathBuf,
        /// Print the complete structured review instead of the bounded text.
        #[arg(long)]
        json: bool,
    },
    /// Persist one explicit human grant for the exact current planning context; no file is edited.
    TaskWriteGrant {
        #[arg(long)]
        state_dir: PathBuf,
        /// Must exactly match the context_id printed by task-write-status.
        #[arg(long)]
        context_id: String,
        #[arg(long)]
        request_key: String,
        #[arg(long, default_value = "human")]
        actor: String,
        /// Explicitly confirm review of the saved proposed paths and limitations.
        #[arg(long, required = true)]
        approve_proposed_paths: bool,
    },
    /// Irreversibly revoke a saved write permission; no file is edited.
    TaskWriteRevoke {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        permission_id: String,
        #[arg(long)]
        request_key: String,
        #[arg(long, default_value = "human")]
        actor: String,
        #[arg(long)]
        reason: String,
    },
    /// Request and apply one bounded patch under the current explicit permission.
    TaskEdit {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        profile: PathBuf,
    },
    /// Run one admitted named verification check through Shuttle's shared executor.
    TaskVerify {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        check_id: String,
        #[arg(long)]
        action_id: String,
        #[arg(long, required = true)]
        approve_host_execution: bool,
    },
    /// Run every unwaived admitted verification check in plan order.
    TaskVerifyAll {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long, required = true)]
        approve_host_execution: bool,
    },
    /// Show admitted verification status without dispatching a check.
    TaskVerifyStatus {
        #[arg(long)]
        state_dir: PathBuf,
    },
    /// Record the fresh passing receipt set for the admitted suite without offering acceptance.
    TaskVerifyEvidence {
        #[arg(long)]
        state_dir: PathBuf,
    },
    /// Create a review-only offer bound to the admitted edit and fresh suite evidence.
    TaskOffer {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        request_key: String,
    },
    /// Inspect the current admitted task's review-only acceptance offer.
    TaskOfferReview {
        #[arg(long)]
        state_dir: PathBuf,
    },
    /// Explicitly accept or reject a task offer from an interactive terminal.
    TaskOfferRespond {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        offer_id: String,
        #[arg(long, value_enum)]
        choice: UserChoice,
    },
    /// Resume ordered native finalization for an accepted admitted task.
    TaskFinalize {
        #[arg(long)]
        state_dir: PathBuf,
    },
    /// Capture a fresh baseline and retire a stale admission without resetting the task run.
    TaskReadmit {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        from_admission: String,
        #[arg(long)]
        request_key: String,
        #[arg(long)]
        reason: String,
    },
    /// Inspect immutable admission history without taking controller ownership.
    TaskAdmissionHistory {
        #[arg(long)]
        state_dir: PathBuf,
    },
}

fn main() -> Result<()> {
    #[cfg(not(windows))]
    let result = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async_main());
    #[cfg(windows)]
    let result = std::thread::Builder::new()
        .name("shuttle-main".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?
                .block_on(async_main())
        })
        .context("start Shuttle runtime")?
        .join()
        .map_err(|_| anyhow::anyhow!("Shuttle runtime panicked"))?;
    result
}

async fn async_main() -> Result<()> {
    #[cfg(target_os = "linux")]
    if std::env::args().nth(1).as_deref() == Some("__process-supervisor") {
        return cortex_shuttle::process::supervise();
    }
    // Keep the large command dispatcher off the Windows main-thread stack. Several
    // commands open async SQLite/native futures; pinning this boundary preserves
    // their behavior without adding a second command path.
    Box::pin(run_command(Box::new(Cli::parse().command))).await
}

async fn run_command(command: Box<Command>) -> Result<()> {
    match *command {
        Command::LlamaProfile {
            endpoint,
            model,
            server_executable,
            model_file,
            output,
            thinking,
        } => {
            let profile =
                LlamaProfile::capture(endpoint, model, &server_executable, &model_file, thinking)
                    .await?;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&output)?;
            use std::io::Write;
            file.write_all(&serde_json::to_vec_pretty(&profile)?)?;
            file.sync_all()?;
            println!("Worker profile captured: {}", output.display());
        }
        Command::LiveRepair {
            profile,
            state_dir,
            python,
            approve_fixture_writes: _,
            approve_host_execution: _,
        } => {
            use cortex_shuttle::model::ModelProvider;
            use std::io::Read;
            let mut bytes = Vec::new();
            std::fs::File::open(profile)?
                .take(8193)
                .read_to_end(&mut bytes)?;
            anyhow::ensure!(bytes.len() <= 8192, "profile exceeds bound");
            let mut model = LlamaModel::new(serde_json::from_slice(&bytes)?)?;
            let fixture = cortex_shuttle::live_repair::initialize(&state_dir.join("fixture"))?;
            let mut journal = Journal::open(&state_dir.join("journal.sqlite")).await?;
            let mut config = AppConfig::default();
            config.database.path = state_dir
                .join("cortexweave.sqlite")
                .to_string_lossy()
                .into_owned();
            let service = CortexWeaveService::open(config).await?;
            let workspace = service
                .register_workspace(
                    fixture.root().to_string_lossy(),
                    "Shuttle qualified live repair",
                )
                .await?;
            journal.ensure_run_with_objective(fixture.root(), &workspace.id, "Repair the isolated value.txt from 41 to exactly 42 followed by LF, with real unittest failure and final verification; await explicit user acceptance.").await?;
            let mut repair = cortex_shuttle::live_repair::LiveRepair::open(
                journal,
                CortexWeaveAdapter::new(service),
                fixture,
                &python,
                model.identity(),
                &std::env::current_exe()?,
            )
            .await?;
            let result: anyhow::Result<()> = tokio::select! {
                result = async {
                    loop {
                        if let Some(offer) = Box::pin(repair.step(&mut model,Fault::None)).await? {
                            println!("{}",serde_json::to_string_pretty(&repair.controller.journal.acceptance_review(&offer.id).await?)?);
                            break Ok(());
                        }
                    }
                } => result,
                signal = tokio::signal::ctrl_c() => { signal?; Err(anyhow::anyhow!("live repair interrupted; unknown work blocks replay on restart")) }
            };
            repair.controller.journal.close().await;
            result?;
        }
        Command::LlamaStep {
            profile,
            state_dir,
            approve_fixture_writes,
        } => {
            use std::io::Read;
            let mut bytes = Vec::new();
            std::fs::File::open(profile)?
                .take(8193)
                .read_to_end(&mut bytes)?;
            anyhow::ensure!(bytes.len() <= 8192, "profile exceeds bound");
            let mut model = LlamaModel::new(serde_json::from_slice(&bytes)?)?;
            let fixture = Fixture::open_or_create(&state_dir.join("fixture"))?;
            let mut journal = Journal::open(&state_dir.join("journal.sqlite")).await?;
            let mut config = AppConfig::default();
            config.database.path = state_dir
                .join("cortexweave.sqlite")
                .to_string_lossy()
                .into_owned();
            let service = CortexWeaveService::open(config).await?;
            let workspace = service
                .register_workspace(
                    fixture.root().to_string_lossy(),
                    "Shuttle local-model development fixture",
                )
                .await?;
            journal.ensure_run(fixture.root(), &workspace.id).await?;
            let mut controller =
                Controller::new(journal, fixture, CortexWeaveAdapter::new(service));
            let bindings = controller.bootstrap(Fault::None).await?;
            let grant = Grant {
                revision: 1,
                fixture_writes: approve_fixture_writes,
                process_authorization_hash: None,
            };
            let result = tokio::select! {
                result = controller.step(
                    &mut model,
                    &grant,
                    &bindings,
                    Fault::None,
                ) => result,
                signal = tokio::signal::ctrl_c() => {
                    signal?;
                    Err(anyhow::anyhow!("model step interrupted; reopen the journal to inspect unknown completion; automatic replay is blocked"))
                }
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&controller.journal.model_requests().await?)?
            );
            controller.journal.close().await;
            result?;
        }
        Command::QualifyEvidence {
            state_dir,
            action_id,
            producer,
        } => {
            let mut journal = open_existing_journal(&state_dir).await?;
            let evidence = if producer == "rustc" {
                journal.qualify_rustc(&action_id).await?
            } else {
                journal.qualify_unittest(&action_id).await?
            };
            let mut config = AppConfig::default();
            config.database.path = state_dir
                .join("cortexweave.sqlite")
                .to_string_lossy()
                .into_owned();
            anyhow::ensure!(
                std::path::Path::new(&config.database.path).is_file(),
                "native state database is missing"
            );
            let sink = CortexWeaveAdapter::new(CortexWeaveService::open(config).await?);
            flush_outbox(&mut journal, &sink, Fault::None).await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "evidence": evidence, "verification": journal.verification_receipt(&action_id).await?
                }))?
            );
            journal.close().await;
        }
        Command::RunStatus { state_dir } => {
            let journal = open_existing_journal(&state_dir).await?;
            let requests: Vec<_> = journal.model_requests().await?.into_iter().map(|r| serde_json::json!({
                "id": r.id, "state": r.state, "applied": r.applied,
                "provider": r.intent.provider, "purpose": r.intent.purpose,
                "elapsed_ms": r.result.as_ref().map(|r| r.elapsed_ms),
                "usage": r.result.as_ref().and_then(|r| r.reply.as_ref()).and_then(|r| r.usage.as_ref()),
                "provider_observation": r.result.as_ref().and_then(|r| r.provider_observation.as_ref()),
                "error": r.result.as_ref().and_then(|r| r.error.as_ref()),
                "limitation": r.result.as_ref().map(|r| &r.limitation),
            })).collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "run": journal.run().await?, "accounting": journal.accounting().await?, "requests": requests,
                }))?
            );
            journal.close().await;
        }
        Command::ResumeStall {
            state_dir,
            request_key,
            reason,
        } => {
            let mut journal = open_existing_journal(&state_dir).await?;
            journal.resume_stall(&request_key, &reason).await?;
            println!("Caller direction saved; run may continue within its existing budgets.");
            journal.close().await;
        }
        Command::Verify {
            workspace,
            state_dir,
            plan,
            check_id,
            action_id,
            approve_host_execution: _,
        } => {
            use std::io::Read;
            let mut bytes = Vec::new();
            std::fs::File::open(plan)?
                .take(65_537)
                .read_to_end(&mut bytes)?;
            anyhow::ensure!(bytes.len() <= 65_536, "verification plan exceeds limit");
            let plan: VerificationPlan = serde_json::from_slice(&bytes)?;
            let revision = plan.revision()?;
            let check = plan
                .checks
                .iter()
                .find(|c| c.id == check_id)
                .context("check ID missing from plan")?;
            let mut journal = Journal::open(&state_dir.join("journal.sqlite")).await?;
            let snapshot = SourceSnapshot::capture(&std::path::absolute(workspace)?, &plan)?;
            let cancellation = Cancellation::default();
            let executor = ProcessExecutor::new(
                &snapshot.workspace_root,
                snapshot.files.iter().map(|f| f.path.clone()).collect(),
                cancellation.clone(),
            )?;
            let grant = Grant {
                revision: 1,
                fixture_writes: false,
                process_authorization_hash: Some(executor.authorization_hash(&check.process)?),
            };
            let mut config = AppConfig::default();
            config.database.path = state_dir
                .join("cortexweave.sqlite")
                .to_string_lossy()
                .into_owned();
            let service = CortexWeaveService::open(config).await?;
            let workspace = service
                .register_workspace(executor.root().to_string_lossy(), "Shuttle verification")
                .await?;
            let run = journal.ensure_run_with_objective(executor.root(), &workspace.id, "Verify an explicitly approved plan and offer its exact state for user acceptance.").await?;
            journal.save_verification_plan(&plan).await?;
            let mut controller =
                Controller::new(journal, executor, CortexWeaveAdapter::new(service));
            let bindings = controller.bootstrap(Fault::None).await?;
            let id = format!("{}/verification/{action_id}", run.id);
            if let Some(existing) = controller.journal.action(&id).await? {
                let binding = controller
                    .journal
                    .verification_binding(&id)
                    .await?
                    .context("saved action is not a verification check")?;
                anyhow::ensure!(
                    binding.plan_revision == revision
                        && binding.check_id == check_id
                        && existing.intent.call == ToolCall::RunProcess(check.process.clone())
                        && existing.intent.grant == grant,
                    "saved verification differs from requested check"
                );
            } else {
                controller
                    .journal
                    .prepare_verification(&id, &revision, &check_id, &controller.executor, &grant)
                    .await?;
            }
            let signal = tokio::spawn(async move {
                if tokio::signal::ctrl_c().await.is_ok() {
                    cancellation.cancel();
                }
            });
            let outcome = controller
                .dispatch(&id, &grant, &bindings, Fault::None)
                .await;
            signal.abort();
            outcome?;
            controller.flush(Fault::None).await?;
            let receipt = controller
                .journal
                .offer_verification_receipt(&id, &revision)
                .await?;
            println!("{}", serde_json::to_string_pretty(&receipt)?);
            println!("Plan revision: {revision}");
            controller.journal.close().await;
            anyhow::ensure!(
                receipt.passed(),
                "verification did not pass for the current state"
            );
        }
        Command::AcceptanceOffer {
            state_dir,
            plan_revision,
        } => {
            let mut journal = open_existing_journal(&state_dir).await?;
            let offer = journal.create_acceptance_offer(&plan_revision).await?;
            print_acceptance_review(&mut journal, &offer.id).await?;
            journal.close().await;
        }
        Command::AcceptanceReview {
            state_dir,
            offer_id,
        } => {
            let mut journal = open_existing_journal(&state_dir).await?;
            print_acceptance_review(&mut journal, &offer_id).await?;
            journal.close().await;
        }
        Command::AcceptanceRespond {
            state_dir,
            offer_id,
            choice,
        } => {
            use std::io::{IsTerminal, Write};
            anyhow::ensure!(
                std::io::stdin().is_terminal(),
                "acceptance requires an explicit response from an interactive terminal"
            );
            let mut journal = open_existing_journal(&state_dir).await?;
            print_acceptance_review(&mut journal, &offer_id).await?;
            let verb = match choice {
                UserChoice::Accept => "accept",
                UserChoice::Reject => "reject",
            };
            let confirmation = format!("{verb} {offer_id}");
            println!(
                "Review the exact evidence, waivers and limitations above. Type {confirmation:?} to confirm, or anything else to cancel:"
            );
            std::io::stdout().flush()?;
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            anyhow::ensure!(
                input.trim() == confirmation,
                "response cancelled; no decision recorded"
            );
            let decision = journal
                .record_user_response(UserResponse {
                    request_key: format!("terminal/{offer_id}/{verb}"),
                    offer_id,
                    choice,
                    user_label: "local interactive terminal user".into(),
                    comment: String::new(),
                })
                .await?;
            println!("{}", serde_json::to_string_pretty(&decision)?);
            journal.close().await;
            if choice == UserChoice::Accept {
                finalize_run(&state_dir).await?;
            }
        }
        Command::Finalize { state_dir } => finalize_run(&state_dir).await?,
        Command::ProcessSpec {
            executable,
            timeout_ms,
            arguments,
        } => {
            let executable = executable.canonicalize()?;
            let mut environment = std::collections::BTreeMap::new();
            if cfg!(windows)
                && let Ok(root) = std::env::var("SystemRoot")
            {
                environment.insert("SystemRoot".into(), root);
            }
            let spec = ProcessSpec {
                executable_hash: hash_executable(&executable)?,
                executable,
                arguments,
                cwd: PathBuf::new(),
                environment,
                limits: ProcessLimits {
                    timeout_ms,
                    ..ProcessLimits::default()
                },
            };
            spec.limits.reservation_ms()?;
            println!("{}", serde_json::to_string_pretty(&spec)?);
        }
        Command::Process {
            workspace,
            state_dir,
            spec,
            input,
            approve_host_execution: _,
        } => {
            use std::io::Read;
            let mut bytes = Vec::new();
            std::fs::File::open(spec)?
                .take(32_769)
                .read_to_end(&mut bytes)?;
            anyhow::ensure!(bytes.len() <= 32_768, "process spec exceeds limit");
            let spec: ProcessSpec = serde_json::from_slice(&bytes)?;
            let cancellation = Cancellation::default();
            let executor = ProcessExecutor::new(
                &std::path::absolute(workspace)?,
                input,
                cancellation.clone(),
            )?;
            let grant = Grant {
                revision: 1,
                fixture_writes: false,
                process_authorization_hash: Some(executor.authorization_hash(&spec)?),
            };
            let mut journal = Journal::open(&state_dir.join("journal.sqlite")).await?;
            let mut config = AppConfig::default();
            config.database.path = state_dir
                .join("cortexweave.sqlite")
                .to_string_lossy()
                .into_owned();
            let service = CortexWeaveService::open(config).await?;
            let workspace_root = executor.root();
            let workspace = service
                .register_workspace(workspace_root.to_string_lossy(), "Shuttle host process")
                .await?;
            let run = journal
                .ensure_run_with_objective(
                    workspace_root,
                    &workspace.id,
                    "Observe one explicitly approved host command.",
                )
                .await?;
            let mut controller =
                Controller::new(journal, executor, CortexWeaveAdapter::new(service));
            let bindings = controller.bootstrap(Fault::None).await?;
            let id = format!("{}/process/0", run.id);
            let intent = if let Some(existing) = controller.journal.action(&id).await? {
                anyhow::ensure!(
                    existing.intent.call == ToolCall::RunProcess(spec.clone())
                        && existing.intent.grant == grant,
                    "saved command differs; use a separate state directory for a newly authorized action"
                );
                existing.intent
            } else {
                ActionIntent {
                    id,
                    call: ToolCall::RunProcess(spec),
                    input_hash: controller.executor.input_hash()?,
                    grant: grant.clone(),
                }
            };
            let signal = tokio::spawn(async move {
                if tokio::signal::ctrl_c().await.is_ok() {
                    cancellation.cancel();
                }
            });
            let outcome = controller
                .submit(intent, &grant, &bindings, Fault::None)
                .await;
            signal.abort();
            let action = outcome?;
            let result = action.result.context("process result missing")?;
            println!(
                "{}",
                String::from_utf8(controller.journal.artifact(&result.artifact_hash).await?)?
            );
            controller.flush(Fault::None).await?;
            println!(
                "Observed {:?}; this is not task acceptance or qualified verification.",
                result.state
            );
            controller.journal.close().await;
            anyhow::ensure!(
                result.state == cortex_shuttle::journal::ActionState::Succeeded,
                "process did not succeed; see the recorded observation"
            );
        }
        Command::Preview {
            state,
            reduced_color,
            export,
            width,
            height,
        } => {
            if let Some(path) = export {
                ui::export_svg(&path, state, width, height)?;
                println!("Preview exported to {}", path.display());
            } else {
                ui::preview(state, reduced_color)?;
            }
        }
        Command::TaskView {
            state_dir,
            reduced_color,
            json,
        } => {
            let reader = TaskReader::open(&state_dir).await?;
            let view = reader.view().await?;
            reader.close().await;
            if json {
                println!("{}", serde_json::to_string_pretty(&view)?);
            } else {
                ui::task_view(view, reduced_color)?;
            }
        }
        Command::Task {
            state_dir,
            root,
            reduced_color,
            profile,
            approve_host_execution,
            workspace: intake_workspace,
            objective: intake_objective,
            constraints: intake_constraints,
            plan: intake_plan,
        } => {
            if intake_workspace.is_some() || intake_objective.is_some() || intake_plan.is_some() {
                let state = state_dir
                    .as_ref()
                    .context("--state-dir is required when creating an intake from shuttle task")?;
                let workspace_root =
                    intake_workspace.context("--workspace is required when creating an intake")?;
                let objective =
                    intake_objective.context("--objective is required when creating an intake")?;
                let plan = intake_plan.context("--plan is required when creating an intake")?;
                let metadata = std::fs::metadata(&plan)?;
                anyhow::ensure!(
                    metadata.len() <= cortex_shuttle::journal::MAX_ARTIFACT_BYTES as u64,
                    "verification plan exceeds 64 KiB"
                );
                let verification_plan: VerificationPlan =
                    serde_json::from_slice(&std::fs::read(&plan)?)?;
                workspace::create_intake(
                    state,
                    &workspace_root,
                    objective,
                    intake_constraints,
                    verification_plan,
                )
                .await?;
            } else {
                anyhow::ensure!(
                    intake_constraints.is_empty(),
                    "--constraint requires --workspace, --objective and --plan"
                );
            }
            let state_dir = if let Some(path) = state_dir {
                path
            } else {
                let tasks = workspace::list_tasks(&root).await?;
                let Some(index) = ui::choose_task(
                    &tasks
                        .iter()
                        .map(|(_, label)| label.clone())
                        .collect::<Vec<_>>(),
                    reduced_color,
                )?
                else {
                    return Ok(());
                };
                tasks[index].0.clone()
            };
            loop {
                let reader = TaskReader::open(&state_dir).await?;
                let view = reader.view().await?;
                let displayed_run = view.run_id.clone();
                let outcome = tokio::task::block_in_place(|| {
                    let handle = tokio::runtime::Handle::current();
                    ui::task_workspace(view, reduced_color, || handle.block_on(reader.view()))
                });
                reader.close().await;
                match outcome? {
                    ui::TaskWorkspaceEvent::Exit => break,
                    ui::TaskWorkspaceEvent::ResumeStall(reason) => {
                        let mut journal = open_existing_journal(&state_dir).await?;
                        anyhow::ensure!(
                            journal.run().await?.context("run missing")?.id == displayed_run,
                            "task changed since direction was reviewed"
                        );
                        let key = format!("terminal/{}/resume", uuid::Uuid::new_v4());
                        let result = journal.resume_stall(&key, &reason).await;
                        journal.close().await;
                        result?;
                    }
                    ui::TaskWorkspaceEvent::ReviewAcceptance(offer_id) => {
                        // A general-task offer is not in the generic offer table.
                        // Show it, with the exact edit, from the task review.
                        let task_offer = workspace::task_acceptance_offer_view(&state_dir)
                            .await
                            .ok()
                            .flatten()
                            .filter(|view| view.offer.id == offer_id);
                        if let Some(view) = task_offer {
                            print_task_offer_review(&view)?;
                            break;
                        }
                        let mut journal = open_existing_journal(&state_dir).await?;
                        let result = print_acceptance_review(&mut journal, &offer_id).await;
                        journal.close().await;
                        result?;
                        break;
                    }
                    ui::TaskWorkspaceEvent::RunScripted(run_id) => {
                        Box::pin(workspace::run_demo(&state_dir, Some(&run_id))).await?;
                    }
                    ui::TaskWorkspaceEvent::General(operation) => {
                        run_terminal_task_operation(
                            &state_dir,
                            operation,
                            profile.as_deref(),
                            approve_host_execution,
                        )
                        .await?;
                    }
                }
            }
        }
        Command::TaskNew { root } => {
            std::fs::create_dir_all(&root)?;
            let path = root.join(format!("task-{}", uuid::Uuid::new_v4()));
            Box::pin(workspace::create_demo(&path)).await?;
            println!(
                "Created scripted fixture task: {}\nOpen with: shuttle task --state-dir \"{}\"",
                path.display(),
                path.display()
            );
        }
        Command::TaskIntake {
            state_dir,
            workspace,
            objective,
            constraints,
            plan,
        } => {
            let metadata = std::fs::metadata(&plan)?;
            anyhow::ensure!(
                metadata.len() <= cortex_shuttle::journal::MAX_ARTIFACT_BYTES as u64,
                "verification plan exceeds 64 KiB"
            );
            let verification_plan: VerificationPlan =
                serde_json::from_slice(&std::fs::read(&plan)?)?;
            let intake = workspace::create_intake(
                &state_dir,
                &workspace,
                objective,
                constraints,
                verification_plan,
            )
            .await?;
            println!(
                "Saved task intake: {}\nVerification plan revision: {}\nOpen with: shuttle task --state-dir \"{}\"",
                intake.id,
                intake.verification_plan_revision,
                state_dir.display()
            );
        }
        Command::TaskPreflight { state_dir } => {
            let reader = TaskReader::open(&state_dir).await?;
            let intake_id = reader.view().await?.run_id;
            reader.close().await;
            let snapshot = workspace::preflight_intake(&state_dir, Some(&intake_id)).await?;
            println!(
                "Saved intake preflight snapshot: {}\nVerification plan revision: {}",
                snapshot.id()?,
                snapshot.plan_revision
            );
        }
        Command::TaskAdmit { state_dir } => {
            let reader = TaskReader::open(&state_dir).await?;
            let intake_id = reader.view().await?.run_id;
            reader.close().await;
            let admission = workspace::admit_intake(&state_dir, Some(&intake_id)).await?;
            println!(
                "Saved general-workflow admission: {}\nPreflight snapshot: {}\nNo executor has been admitted.",
                admission.id, admission.preflight_snapshot
            );
        }
        Command::TaskPlan { state_dir, profile } => {
            use std::io::Read;
            let mut bytes = Vec::new();
            std::fs::File::open(profile)?
                .take(8193)
                .read_to_end(&mut bytes)?;
            anyhow::ensure!(bytes.len() <= 8192, "profile exceeds bound");
            let context = workspace::capture_admitted_task_context(&state_dir).await?;
            let mut model = LlamaModel::for_admitted_planning(
                serde_json::from_slice(&bytes)?,
                context.clone(),
            )?;
            let mut config = AppConfig::default();
            config.database.path = state_dir
                .join("cortexweave.sqlite")
                .to_string_lossy()
                .into_owned();
            let service = CortexWeaveService::open(config).await?;
            let native_workspace = service
                .register_workspace(&context.workspace_root, "Shuttle admitted task planning")
                .await?;
            let view = workspace::run_admitted_task_planning(
                &state_dir,
                &native_workspace.id,
                &context,
                &mut model,
            )
            .await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "context_id": context.id()?,
                    "admission_id": context.admission_id,
                    "snapshot_id": context.snapshot_id,
                    "declared_files": context.files.len(),
                    "proposal": view.proposal,
                    "stale_reason": view.stale_reason,
                }))?
            );
        }
        Command::TaskWriteStatus { state_dir } => {
            let reader = TaskReader::open(&state_dir).await?;
            let view = reader.write_permission_view().await?;
            reader.close().await;
            println!("{}", serde_json::to_string_pretty(&view)?);
        }
        Command::TaskEditReview { state_dir, json } => {
            let reader = TaskReader::open(&state_dir).await?;
            let review = reader.edit_review().await?;
            let legacy = reader.legacy_edit_requests().await?;
            reader.close().await;
            match (review, json) {
                (Some(review), true) => println!(
                    "{}",
                    edit_review::json_terminal_safe(&serde_json::to_string_pretty(&review)?)
                ),
                (Some(review), false) => review.lines().iter().for_each(|line| println!("{line}")),
                // JSON `null` means only "no v2 session and no edit action".
                (None, true) => println!("null"),
                (None, false) => println!("{}", edit_review::no_edit_review_line(legacy)),
            }
        }
        Command::TaskWriteGrant {
            state_dir,
            context_id,
            request_key,
            actor,
            approve_proposed_paths,
        } => {
            anyhow::ensure!(
                approve_proposed_paths,
                "explicit proposed-path approval is required"
            );
            let view = workspace::grant_task_write_permission(
                &state_dir,
                &context_id,
                &request_key,
                &actor,
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&view)?);
        }
        Command::TaskWriteRevoke {
            state_dir,
            permission_id,
            request_key,
            actor,
            reason,
        } => {
            let revocation = workspace::revoke_task_write_permission(
                &state_dir,
                &permission_id,
                &request_key,
                &actor,
                &reason,
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&revocation)?);
        }
        Command::TaskEdit { state_dir, profile } => {
            use std::io::Read;
            let reader = TaskReader::open(&state_dir).await?;
            let view = reader.write_permission_view().await?;
            reader.close().await;
            view.permission
                .as_ref()
                .context("explicit write permission is required")?;
            anyhow::ensure!(
                view.unusable_reason.is_none(),
                "write permission is unusable"
            );
            let mut bytes = Vec::new();
            std::fs::File::open(profile)?
                .take(8193)
                .read_to_end(&mut bytes)?;
            anyhow::ensure!(bytes.len() <= 8192, "profile exceeds bound");
            let profile: LlamaProfile = serde_json::from_slice(&bytes)?;
            let mut config = AppConfig::default();
            config.database.path = state_dir
                .join("cortexweave.sqlite")
                .to_string_lossy()
                .into_owned();
            let service = CortexWeaveService::open(config).await?;
            let native_workspace = service
                .register_workspace(&view.admission.workspace_root, "Shuttle admitted task edit")
                .await?;
            let action = workspace::run_admitted_task_edit(
                &state_dir,
                &native_workspace.id,
                &profile.digest()?,
                |session| LlamaModel::for_admitted_editing_v2(profile.clone(), session.id.clone()),
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&action)?);
        }
        Command::TaskVerify {
            state_dir,
            check_id,
            action_id,
            approve_host_execution: _,
        } => {
            let (admission, plan) = workspace::load_admitted_plan(&state_dir, None).await?;
            let receipt =
                run_admitted_check(&state_dir, &admission, &plan, &check_id, &action_id).await?;
            println!("{}", serde_json::to_string_pretty(&receipt)?);
            anyhow::ensure!(
                receipt.passed(),
                "verification did not pass for the current state"
            );
        }
        Command::TaskVerifyAll {
            state_dir,
            approve_host_execution: _,
        } => {
            let (_, initial_plan) = workspace::load_admitted_plan(&state_dir, None).await?;
            let mut receipts = Vec::new();
            for check in initial_plan.checks {
                if initial_plan
                    .waivers
                    .iter()
                    .any(|waiver| waiver.check_id == check.id)
                {
                    continue;
                }
                // Re-load before every dispatch so a change between checks blocks
                // the remaining suite rather than lending old evidence new state.
                let (admission, plan) = workspace::load_admitted_plan(&state_dir, None).await?;
                let journal = Journal::open(&state_dir.join("journal.sqlite")).await?;
                let saved_action =
                    workspace::admission_check_action(&journal, &admission, &check.id).await?;
                journal.close().await;
                let action_id =
                    saved_action.unwrap_or_else(|| admitted_suite_action_id(&admission, &check.id));
                let receipt =
                    run_admitted_check(&state_dir, &admission, &plan, &check.id, &action_id)
                        .await?;
                receipts.push(receipt);
            }
            // Later checks may have invalidated earlier observations.
            let mut journal = Journal::open(&state_dir.join("journal.sqlite")).await?;
            for receipt in &mut receipts {
                *receipt = journal
                    .verification_receipt(&receipt.receipt.action_id)
                    .await?
                    .context("suite receipt missing")?;
            }
            journal.close().await;
            println!("{}", serde_json::to_string_pretty(&receipts)?);
            anyhow::ensure!(
                receipts.iter().all(ReceiptView::passed),
                "one or more admitted verification checks did not pass"
            );
        }
        Command::TaskVerifyStatus { state_dir } => {
            let (admission, plan) = workspace::load_saved_admitted_plan(&state_dir, None).await?;
            let mut journal = Journal::open(&state_dir.join("journal.sqlite")).await?;
            let run = journal.run().await?;
            let mut statuses = Vec::new();
            for check in &plan.checks {
                let waiver_reason = plan
                    .waivers
                    .iter()
                    .find(|waiver| waiver.check_id == check.id)
                    .map(|waiver| waiver.reason.clone());
                if waiver_reason.is_some() {
                    statuses.push(AdmittedCheckStatus {
                        check_id: check.id.clone(),
                        name: check.name.clone(),
                        status: "waived".into(),
                        action_id: None,
                        waiver_reason,
                        receipt: None,
                    });
                    continue;
                }
                let action_id =
                    workspace::admission_check_action(&journal, &admission, &check.id).await?;
                let (receipt, action_state) = match (&run, &action_id) {
                    (Some(run), Some(action_id)) => {
                        let id = format!("{}/verification/{action_id}", run.id);
                        (
                            journal.verification_receipt(&id).await?,
                            journal.action(&id).await?.map(|action| action.state),
                        )
                    }
                    _ => (None, None),
                };
                let status = admitted_check_status(
                    receipt
                        .as_ref()
                        .map(|view| (view.stale_reason.is_some(), view.passed())),
                    action_state,
                    action_id.is_some(),
                );
                statuses.push(AdmittedCheckStatus {
                    check_id: check.id.clone(),
                    name: check.name.clone(),
                    status: status.into(),
                    action_id,
                    waiver_reason: None,
                    receipt,
                });
            }
            let evidence = journal.suite_evidence(&admission.id).await?;
            journal.close().await;
            println!(
                "{}",
                serde_json::to_string_pretty(&AdmittedSuiteStatus {
                    checks: statuses,
                    evidence,
                })?
            );
        }
        Command::TaskVerifyEvidence { state_dir } => {
            let (admission, plan) = workspace::load_admitted_plan(&state_dir, None).await?;
            let mut journal = Journal::open(&state_dir.join("journal.sqlite")).await?;
            let run = journal.run().await?.context("admitted run missing")?;
            let mut actions = Vec::new();
            for check in &plan.checks {
                if plan
                    .waivers
                    .iter()
                    .any(|waiver| waiver.check_id == check.id)
                {
                    continue;
                }
                let action = workspace::admission_check_action(&journal, &admission, &check.id)
                    .await?
                    .context("required admitted check has no action binding")?;
                actions.push((
                    check.id.clone(),
                    format!("{}/verification/{action}", run.id),
                ));
            }
            let evidence = journal
                .record_suite_evidence(
                    &admission.id,
                    &admission.verification_plan_revision,
                    &admission.preflight_snapshot,
                    &actions,
                )
                .await?;
            journal.close().await;
            println!("{}", serde_json::to_string_pretty(&evidence)?);
        }
        Command::TaskOffer {
            state_dir,
            request_key,
        } => {
            let offer = workspace::offer_task_acceptance(&state_dir, &request_key).await?;
            println!("{}", serde_json::to_string_pretty(&offer)?);
        }
        Command::TaskOfferReview { state_dir } => {
            let offer = workspace::task_acceptance_offer_view(&state_dir).await?;
            println!("{}", serde_json::to_string_pretty(&offer)?);
        }
        Command::TaskOfferRespond {
            state_dir,
            offer_id,
            choice,
        } => {
            use std::io::{IsTerminal, Write};
            anyhow::ensure!(
                std::io::stdin().is_terminal(),
                "task acceptance requires an explicit response from an interactive terminal"
            );
            let view = workspace::task_acceptance_offer_view(&state_dir)
                .await?
                .context("task acceptance offer missing")?;
            anyhow::ensure!(
                view.offer.id == offer_id,
                "task acceptance response names another offer"
            );
            print_task_offer_review(&view)?;
            let verb = match choice {
                UserChoice::Accept => "accept",
                UserChoice::Reject => "reject",
            };
            let confirmation = format!("{verb} {offer_id}");
            println!(
                "Review the exact task edit, suite evidence and limitations above. Type {confirmation:?} to confirm, or anything else to cancel:"
            );
            std::io::stdout().flush()?;
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            anyhow::ensure!(
                input.trim() == confirmation,
                "response cancelled; no task decision recorded"
            );
            let decision = workspace::record_task_acceptance_response(
                &state_dir,
                UserResponse {
                    request_key: format!("terminal/task/{offer_id}/{verb}"),
                    offer_id,
                    choice,
                    user_label: "local interactive terminal user".into(),
                    comment: String::new(),
                },
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&decision)?);
            if choice == UserChoice::Accept {
                finalize_run(&state_dir).await?;
            }
        }
        Command::TaskFinalize { state_dir } => finalize_run(&state_dir).await?,
        Command::TaskReadmit {
            state_dir,
            from_admission,
            request_key,
            reason,
        } => {
            let admission =
                workspace::readmit_intake(&state_dir, &from_admission, &request_key, &reason)
                    .await?;
            println!("{}", serde_json::to_string_pretty(&admission)?);
        }
        Command::TaskAdmissionHistory { state_dir } => {
            println!(
                "{}",
                serde_json::to_string_pretty(&workspace::admission_history(&state_dir).await?)?
            );
        }
        Command::Demo { state_dir } => {
            Box::pin(workspace::run_demo(&state_dir, None)).await?;
            let reader = TaskReader::open(&state_dir).await?;
            println!("{}", serde_json::to_string_pretty(&reader.view().await?)?);
            reader.close().await;
        }
    }
    Ok(())
}

/// Execute one terminal-confirmed general-task operation through the same
/// workspace functions as the headless CLI.  The terminal has already been
/// restored when this runs; every operation revalidates durable state itself.
async fn run_terminal_task_operation(
    state_dir: &std::path::Path,
    operation: TaskOperation,
    profile: Option<&std::path::Path>,
    approve_host_execution: bool,
) -> Result<()> {
    match operation {
        TaskOperation::Preflight => {
            let reader = TaskReader::open(state_dir).await?;
            let intake_id = reader.view().await?.run_id;
            reader.close().await;
            workspace::preflight_intake(state_dir, Some(&intake_id)).await?;
        }
        TaskOperation::Admit => {
            let reader = TaskReader::open(state_dir).await?;
            let intake_id = reader.view().await?.run_id;
            reader.close().await;
            workspace::admit_intake(state_dir, Some(&intake_id)).await?;
        }
        TaskOperation::Plan => {
            use std::io::Read;
            let profile = profile.context("--profile is required for Ctrl+L planning")?;
            let mut bytes = Vec::new();
            std::fs::File::open(profile)?
                .take(8193)
                .read_to_end(&mut bytes)?;
            anyhow::ensure!(bytes.len() <= 8192, "profile exceeds bound");
            let context = workspace::capture_admitted_task_context(state_dir).await?;
            let mut model = LlamaModel::for_admitted_planning(
                serde_json::from_slice(&bytes)?,
                context.clone(),
            )?;
            let mut config = AppConfig::default();
            config.database.path = state_dir
                .join("cortexweave.sqlite")
                .to_string_lossy()
                .into_owned();
            let service = CortexWeaveService::open(config).await?;
            let native_workspace = service
                .register_workspace(&context.workspace_root, "Shuttle admitted task planning")
                .await?;
            workspace::run_admitted_task_planning(
                state_dir,
                &native_workspace.id,
                &context,
                &mut model,
            )
            .await?;
        }
        TaskOperation::Grant => {
            let reader = TaskReader::open(state_dir).await?;
            let view = reader.write_permission_view().await?;
            reader.close().await;
            workspace::grant_task_write_permission(
                state_dir,
                &view.context.id()?,
                &format!("terminal/{}/grant", uuid::Uuid::new_v4()),
                "local terminal user",
            )
            .await?;
        }
        TaskOperation::Edit => {
            use std::io::Read;
            let profile = profile.context("--profile is required for Ctrl+E editing")?;
            let reader = TaskReader::open(state_dir).await?;
            let view = reader.write_permission_view().await?;
            reader.close().await;
            view.permission
                .as_ref()
                .context("explicit write permission is required")?;
            anyhow::ensure!(
                view.unusable_reason.is_none(),
                "write permission is unusable"
            );
            let mut bytes = Vec::new();
            std::fs::File::open(profile)?
                .take(8193)
                .read_to_end(&mut bytes)?;
            anyhow::ensure!(bytes.len() <= 8192, "profile exceeds bound");
            let profile: LlamaProfile = serde_json::from_slice(&bytes)?;
            let mut config = AppConfig::default();
            config.database.path = state_dir
                .join("cortexweave.sqlite")
                .to_string_lossy()
                .into_owned();
            let service = CortexWeaveService::open(config).await?;
            let native_workspace = service
                .register_workspace(&view.admission.workspace_root, "Shuttle admitted task edit")
                .await?;
            workspace::run_admitted_task_edit(
                state_dir,
                &native_workspace.id,
                &profile.digest()?,
                |session| LlamaModel::for_admitted_editing_v2(profile.clone(), session.id.clone()),
            )
            .await?;
        }
        TaskOperation::Verify => {
            anyhow::ensure!(
                approve_host_execution,
                "--approve-host-execution is required for Ctrl+V verification"
            );
            let (_, initial_plan) = workspace::load_admitted_plan(state_dir, None).await?;
            for check in initial_plan.checks {
                if initial_plan
                    .waivers
                    .iter()
                    .any(|waiver| waiver.check_id == check.id)
                {
                    continue;
                }
                let (admission, plan) = workspace::load_admitted_plan(state_dir, None).await?;
                let journal = Journal::open(&state_dir.join("journal.sqlite")).await?;
                let saved =
                    workspace::admission_check_action(&journal, &admission, &check.id).await?;
                journal.close().await;
                let action_id =
                    saved.unwrap_or_else(|| admitted_suite_action_id(&admission, &check.id));
                let receipt =
                    run_admitted_check(state_dir, &admission, &plan, &check.id, &action_id).await?;
                anyhow::ensure!(
                    receipt.passed(),
                    "verification did not pass for the current state"
                );
            }
        }
        TaskOperation::Offer => {
            workspace::offer_task_acceptance(
                state_dir,
                &format!("terminal/{}/offer", uuid::Uuid::new_v4()),
            )
            .await?;
        }
        TaskOperation::Accept | TaskOperation::Reject => {
            let view = workspace::task_acceptance_offer_view(state_dir)
                .await?
                .context("task acceptance offer missing")?;
            let choice = if operation == TaskOperation::Accept {
                UserChoice::Accept
            } else {
                UserChoice::Reject
            };
            workspace::record_task_acceptance_response(
                state_dir,
                UserResponse {
                    request_key: format!("terminal/task/{}/{choice:?}", view.offer.id),
                    offer_id: view.offer.id,
                    choice,
                    user_label: "local terminal user".into(),
                    comment: String::new(),
                },
            )
            .await?;
            if choice == UserChoice::Accept {
                finalize_run(state_dir).await?;
            }
        }
        TaskOperation::Finalize => finalize_run(state_dir).await?,
        TaskOperation::Readmit => {
            let (admission, _) = workspace::load_saved_admitted_plan(state_dir, None).await?;
            workspace::readmit_intake(
                state_dir,
                &admission.id,
                &format!("terminal/{}/readmit", uuid::Uuid::new_v4()),
                "Terminal requested re-admission after reviewed state changed.",
            )
            .await?;
        }
    }
    Ok(())
}

async fn run_admitted_check(
    state_dir: &std::path::Path,
    admission: &TaskAdmission,
    plan: &VerificationPlan,
    check_id: &str,
    action_id: &str,
) -> Result<ReceiptView> {
    let check = plan
        .checks
        .iter()
        .find(|check| check.id == check_id)
        .cloned()
        .context("check ID missing from admitted plan")?;
    anyhow::ensure!(
        !plan
            .waivers
            .iter()
            .any(|waiver| waiver.check_id == check_id),
        "waived checks cannot be dispatched"
    );
    let snapshot = SourceSnapshot::capture(std::path::Path::new(&admission.workspace_root), plan)?;
    anyhow::ensure!(
        snapshot.id()? == admission.preflight_snapshot,
        "declared inputs changed since admission"
    );
    let executor = ProcessExecutor::new(
        &snapshot.workspace_root,
        snapshot
            .files
            .iter()
            .map(|file| file.path.clone())
            .collect(),
        Cancellation::default(),
    )?;
    let grant = Grant {
        revision: 1,
        fixture_writes: false,
        process_authorization_hash: Some(executor.authorization_hash(&check.process)?),
    };
    let mut journal = Journal::open(&state_dir.join("journal.sqlite")).await?;
    let mut config = AppConfig::default();
    config.database.path = state_dir
        .join("cortexweave.sqlite")
        .to_string_lossy()
        .into_owned();
    let service = CortexWeaveService::open(config).await?;
    let native_workspace = service
        .register_workspace(
            executor.root().to_string_lossy(),
            "Shuttle admitted verification",
        )
        .await?;
    let objective = journal.run().await?.map_or_else(
        || "Run admitted verification suite.".to_string(),
        |run| run.objective,
    );
    let run = journal
        .ensure_run_with_objective(executor.root(), &native_workspace.id, &objective)
        .await?;
    workspace::bind_admission_check_for_run(&journal, admission, &run.id, check_id, action_id)
        .await?;
    journal.resume_after_failed_verification_check().await?;
    journal.save_verification_plan(plan).await?;
    let mut controller = Controller::new(journal, executor, CortexWeaveAdapter::new(service));
    let bindings = controller.bootstrap(Fault::None).await?;
    let id = format!("{}/verification/{action_id}", run.id);
    controller
        .journal
        .prepare_verification(
            &id,
            &admission.verification_plan_revision,
            check_id,
            &controller.executor,
            &grant,
        )
        .await?;
    controller
        .dispatch(&id, &grant, &bindings, Fault::None)
        .await?;
    controller.flush(Fault::None).await?;
    let receipt = controller
        .journal
        .offer_verification_receipt(&id, &admission.verification_plan_revision)
        .await?;
    controller.journal.close().await;
    Ok(receipt)
}

/// The label for one unwaived check of an admitted suite. A binding is recorded
/// before its action is prepared, so a refusal during preparation leaves a binding
/// with no action: nothing was started. A prepared action was likewise never
/// started, so neither is an unknown effect; only a started or unknown action is.
/// `receipt` is `(stale, passed)`.
fn admitted_check_status(
    receipt: Option<(bool, bool)>,
    action: Option<cortex_shuttle::journal::ActionState>,
    bound: bool,
) -> &'static str {
    use cortex_shuttle::journal::ActionState;
    match (receipt, action) {
        (Some((true, _)), _) => "stale",
        (Some((false, true)), _) => "passed",
        (Some((false, false)), _) => "failed",
        (None, Some(ActionState::Prepared)) => "not_started",
        (None, Some(_)) => "unknown",
        (None, None) if bound => "not_prepared",
        (None, None) => "pending",
    }
}

fn admitted_suite_action_id(admission: &TaskAdmission, check_id: &str) -> String {
    let digest = blake3::hash(check_id.as_bytes()).to_hex();
    format!("suite/{}/{}", admission.id, digest)
}

async fn open_existing_journal(state_dir: &std::path::Path) -> Result<Journal> {
    let path = state_dir.join("journal.sqlite");
    anyhow::ensure!(path.is_file(), "an existing Shuttle journal is required");
    Journal::open(&path).await
}

/// The exact edit first, as bounded text, then the complete offer as JSON.
fn print_task_offer_review(view: &workspace::TaskAcceptanceOfferView) -> Result<()> {
    if let Some(change) = &view.change {
        change.lines().iter().for_each(|line| println!("{line}"));
        println!();
    }
    println!(
        "{}",
        edit_review::json_terminal_safe(&serde_json::to_string_pretty(view)?)
    );
    Ok(())
}

async fn print_acceptance_review(journal: &mut Journal, offer_id: &str) -> Result<()> {
    // JSON escapes C0 controls; `json_terminal_safe` covers the rest.
    println!(
        "{}",
        edit_review::json_terminal_safe(&serde_json::to_string_pretty(
            &journal.acceptance_review(offer_id).await?
        )?)
    );
    Ok(())
}

async fn finalize_run(state_dir: &std::path::Path) -> Result<()> {
    let mut journal = open_existing_journal(state_dir).await?;
    let run = journal.run().await?.context("run missing")?;
    anyhow::ensure!(
        matches!(run.phase.as_str(), "finalizing" | "finalized"),
        "an explicitly accepted run is required"
    );
    let mut config = AppConfig::default();
    config.database.path = state_dir
        .join("cortexweave.sqlite")
        .to_string_lossy()
        .into_owned();
    anyhow::ensure!(
        std::path::Path::new(&config.database.path).is_file(),
        "native state database is missing"
    );
    let sink = CortexWeaveAdapter::new(CortexWeaveService::open(config).await?);
    flush_outbox(&mut journal, &sink, Fault::None).await?;
    println!("{}", journal.run().await?.context("run missing")?.phase);
    journal.close().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::admitted_check_status;
    use cortex_shuttle::journal::ActionState;

    #[test]
    fn a_check_is_unknown_only_when_its_action_started() {
        assert_eq!(admitted_check_status(None, None, false), "pending");
        assert_eq!(admitted_check_status(None, None, true), "not_prepared");
        assert_eq!(
            admitted_check_status(None, Some(ActionState::Prepared), true),
            "not_started"
        );
        for state in [ActionState::Started, ActionState::Unknown] {
            assert_eq!(admitted_check_status(None, Some(state), true), "unknown");
        }
        assert_eq!(
            admitted_check_status(Some((true, true)), Some(ActionState::Succeeded), true),
            "stale"
        );
        assert_eq!(
            admitted_check_status(Some((false, true)), Some(ActionState::Succeeded), true),
            "passed"
        );
        assert_eq!(
            admitted_check_status(Some((false, false)), Some(ActionState::Failed), true),
            "failed"
        );
    }
}
