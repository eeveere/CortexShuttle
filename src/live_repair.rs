//! One isolated live task with real failure/final verification around model edits.
use crate::{
    acceptance::AcceptanceOffer,
    adapter::NativeSink,
    controller::{Controller, Fault, Observation, ToolExecutor},
    evidence::unittest_output,
    fixture::Fixture,
    journal::{ActionIntent, Grant, Journal, transition},
    model::ModelProvider,
    process::{Cancellation, ProcessExecutor, ProcessLimits, ProcessSpec, hash_executable},
    verification::{DeclaredInput, InputKind, SourceSnapshot, VerificationCheck, VerificationPlan},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const TEST: &str = "import unittest\nfrom pathlib import Path\nclass FixtureContract(unittest.TestCase):\n    def test_exact_value(self):\n        self.assertEqual(Path('value.txt').read_bytes(), b'42\\n')\n";
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Configuration {
    version: u32,
    provider: String,
    baseline: VerificationPlan,
    final_check: VerificationPlan,
}

pub enum LiveExecutor {
    Fixture(Fixture),
    Process(ProcessExecutor),
}
#[async_trait::async_trait]
impl ToolExecutor for LiveExecutor {
    fn input_hash(&self) -> Result<String> {
        match self {
            Self::Fixture(e) => e.input_hash(),
            Self::Process(e) => e.input_hash(),
        }
    }
    fn validate(&self, intent: &ActionIntent, grant: &Grant) -> Result<()> {
        match self {
            Self::Fixture(e) => {
                initialize(e.root())?;
                e.validate(intent, grant)
            }
            Self::Process(e) => e.validate(intent, grant),
        }
    }
    fn execute(&mut self, intent: &ActionIntent) -> Result<Observation> {
        match self {
            Self::Fixture(e) => {
                initialize(e.root())?;
                e.execute(intent)
            }
            Self::Process(e) => e.execute(intent),
        }
    }
    async fn execute_async(&mut self, intent: &ActionIntent) -> Result<Observation> {
        match self {
            Self::Fixture(e) => {
                initialize(e.root())?;
                e.execute_async(intent).await
            }
            Self::Process(e) => e.execute_async(intent).await,
        }
    }
    fn mode(&self) -> &'static str {
        "qualified_live_fixture_v1"
    }
}
pub struct LiveRepair<S> {
    pub controller: Controller<LiveExecutor, S>,
    root: PathBuf,
    config: Configuration,
    supervisor: PathBuf,
}

/// Initialization creates only a new isolated fixture; later opens validate files.
pub fn initialize(root: &Path) -> Result<Fixture> {
    let new = !root.exists();
    let fixture = Fixture::open_or_create(root)?;
    for (name, bytes) in [
        ("test_probe.py", TEST.as_bytes()),
        (
            "capture_unittest.py",
            include_bytes!("../integrations/unittest/capture_unittest.py").as_slice(),
        ),
        (
            "shuttle_capture.py",
            include_bytes!("../integrations/unittest/shuttle_capture.py").as_slice(),
        ),
    ] {
        let path = fixture.root().join(name);
        if new {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        crate::process::check_absolute_path(&path)?;
        ensure!(
            fs::metadata(&path)?.len() == bytes.len() as u64,
            "live fixture test/runner size changed"
        );
        let mut actual = Vec::new();
        fs::File::open(path)?
            .take(bytes.len() as u64 + 1)
            .read_to_end(&mut actual)?;
        ensure!(
            actual == bytes,
            "live fixture test/runner definition changed"
        );
    }
    Ok(fixture)
}
fn plan(python: &Path, action: &str) -> Result<VerificationPlan> {
    let mut environment = BTreeMap::new();
    for key in ["SystemRoot", "LD_LIBRARY_PATH"] {
        if let Ok(value) = std::env::var(key) {
            environment.insert(key.into(), value);
        }
    }
    let arguments = vec![
        "-I".into(),
        "shuttle_capture.py".into(),
        "test_probe".into(),
        "--workspace".into(),
        ".".into(),
        "--output".into(),
        unittest_output(action),
        "--run-id".into(),
        action.into(),
    ];
    Ok(VerificationPlan {
        version: 1,
        checks: vec![VerificationCheck {
            id: "unittest".into(),
            name: "value.txt is exactly 42 followed by LF".into(),
            process: ProcessSpec {
                executable: python.into(),
                executable_hash: hash_executable(python)?,
                arguments,
                cwd: PathBuf::new(),
                environment,
                limits: ProcessLimits {
                    timeout_ms: 30_000,
                    ..ProcessLimits::default()
                },
            },
        }],
        inputs: vec![
            DeclaredInput {
                path: "value.txt".into(),
                kind: InputKind::Source,
            },
            DeclaredInput {
                path: "fixture.manifest".into(),
                kind: InputKind::RunnerConfiguration,
            },
            DeclaredInput {
                path: "test_probe.py".into(),
                kind: InputKind::TestDefinition,
            },
            DeclaredInput {
                path: "capture_unittest.py".into(),
                kind: InputKind::RunnerConfiguration,
            },
            DeclaredInput {
                path: "shuttle_capture.py".into(),
                kind: InputKind::RunnerConfiguration,
            },
        ],
        exclusions: vec![],
        waivers: vec![],
    })
}
impl<S: NativeSink> LiveRepair<S> {
    pub async fn open(
        mut journal: Journal,
        sink: S,
        fixture: Fixture,
        python: &Path,
        provider: &str,
        supervisor: &Path,
    ) -> Result<Self> {
        let run = journal.run().await?.context("initialize run first")?;
        ensure!(
            Path::new(&run.workspace_root) == fixture.root(),
            "live workspace mismatch"
        );
        let config = Configuration {
            version: 1,
            provider: provider.into(),
            baseline: plan(python, &format!("{}/live/baseline", run.id))?,
            final_check: plan(python, &format!("{}/live/final", run.id))?,
        };
        let bytes = crate::verification::bounded_json(&config)?;
        let saved: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT configuration FROM live_repairs WHERE singleton = 1")
                .fetch_optional(&journal.pool)
                .await?;
        if let Some(saved) = saved {
            ensure!(
                saved == bytes,
                "live repair configuration changed; existing run cannot be repurposed"
            );
        } else {
            ensure!(
                journal.actions().await?.is_empty() && journal.model_requests().await?.is_empty(),
                "live repair requires a fresh run"
            );
            sqlx::query("INSERT INTO live_repairs(singleton, configuration, phase) VALUES (1, ?, 'baseline')").bind(bytes).execute(&journal.pool).await?;
        }
        journal.save_verification_plan(&config.baseline).await?;
        journal.save_verification_plan(&config.final_check).await?;
        let root = fixture.root().to_owned();
        Ok(Self {
            controller: Controller::new(journal, LiveExecutor::Fixture(fixture), sink),
            root,
            config,
            supervisor: supervisor.into(),
        })
    }
    /// At most one model step; harness verification stages reuse the process executor.
    pub async fn step(
        &mut self,
        model: &mut impl ModelProvider,
        fault: Fault,
    ) -> Result<Option<AcceptanceOffer>> {
        let lease = self
            .controller
            .journal
            .begin_activity("live_repair_step", true)
            .await?;
        let result = Box::pin(self.step_inner(model, fault)).await;
        self.controller.journal.finish_activity(lease, result).await
    }
    async fn step_inner(
        &mut self,
        model: &mut impl ModelProvider,
        fault: Fault,
    ) -> Result<Option<AcceptanceOffer>> {
        ensure!(
            model.identity() == self.config.provider,
            "live provider changed"
        );
        initialize(&self.root)?;
        let bindings = Box::pin(self.controller.bootstrap(Fault::None)).await?;
        let run = self
            .controller
            .journal
            .run()
            .await?
            .context("run missing")?;
        if matches!(
            run.phase.as_str(),
            "awaiting_acceptance" | "finalizing" | "finalized"
        ) {
            let offer: String = sqlx::query_scalar(
                "SELECT active_acceptance_offer_id FROM runs WHERE singleton = 1",
            )
            .fetch_one(&self.controller.journal.pool)
            .await?;
            return Ok(Some(
                self.controller
                    .journal
                    .acceptance_review(&offer)
                    .await?
                    .view
                    .offer,
            ));
        }
        let phase: String =
            sqlx::query_scalar("SELECT phase FROM live_repairs WHERE singleton = 1")
                .fetch_one(&self.controller.journal.pool)
                .await?;
        if phase == "model" {
            self.controller.executor = LiveExecutor::Fixture(initialize(&self.root)?);
            if Box::pin(self.controller.step(
                model,
                &Grant {
                    revision: 1,
                    fixture_writes: true,
                    process_authorization_hash: None,
                },
                &bindings,
                fault,
            ))
            .await?
            {
                let mut tx = self.controller.journal.pool.begin().await?;
                sqlx::query("UPDATE live_repairs SET phase = 'final' WHERE singleton = 1 AND phase = 'model'").execute(&mut *tx).await?;
                transition(
                    &mut tx,
                    "ready",
                    "Model requested review; a fresh qualified check is still required",
                )
                .await?;
                tx.commit().await?;
            }
            return Ok(None);
        }
        let baseline = phase == "baseline";
        ensure!(baseline || phase == "final", "invalid live phase");
        let plan = if baseline {
            self.config.baseline.clone()
        } else {
            self.config.final_check.clone()
        };
        let id = format!(
            "{}/live/{}",
            run.id,
            if baseline { "baseline" } else { "final" }
        );
        let snapshot = SourceSnapshot::capture(&self.root, &plan)?;
        let executor = ProcessExecutor::new(
            &self.root,
            snapshot.files.iter().map(|f| f.path.clone()).collect(),
            Cancellation::default(),
        )?
        .with_supervisor(self.supervisor.clone())?;
        let grant = Grant {
            revision: 1,
            fixture_writes: false,
            process_authorization_hash: Some(executor.authorization_hash(&plan.checks[0].process)?),
        };
        if self.controller.journal.action(&id).await?.is_none() {
            self.controller
                .journal
                .prepare_verification(&id, &plan.revision()?, "unittest", &executor, &grant)
                .await?;
        }
        self.controller.executor = LiveExecutor::Process(executor);
        Box::pin(self.controller.dispatch(&id, &grant, &bindings, fault)).await?;
        self.controller.flush(fault).await?;
        let evidence = self.controller.journal.qualify_unittest(&id).await?;
        self.controller.flush(fault).await?;
        if baseline {
            ensure!(
                self.controller
                    .journal
                    .verification_receipt(&id)
                    .await?
                    .context("baseline receipt missing")?
                    .stale_reason
                    .is_none(),
                "baseline changed before continuation"
            );
            ensure!(
                evidence.payload["exit_code"] == 1
                    && evidence.payload["counts"]["parents"]["assertion_failed"] == 1,
                "expected real initial assertion failure; no model write authorized for another outcome"
            );
            self.controller.journal.ensure_execution_budget().await?;
            self.controller.journal.ensure_no_pending_model().await?;
            ensure!(
                self.controller.journal.actions().await?.len() == 1,
                "baseline continuation has unrelated actions"
            );
            let mut tx = self.controller.journal.pool.begin().await?;
            let changed = sqlx::query("UPDATE live_repairs SET phase = 'model' WHERE singleton = 1 AND phase = 'baseline'").execute(&mut *tx).await?;
            ensure!(changed.rows_affected() == 1, "baseline already consumed");
            transition(&mut tx,"ready","Expected qualified baseline failure observed; continue the authorized isolated repair without resetting budgets").await?;
            tx.commit().await?;
            Ok(None)
        } else {
            ensure!(
                evidence.payload["exit_code"] == 0,
                "final qualified check failed; user acceptance is blocked"
            );
            Ok(Some(
                self.controller
                    .journal
                    .create_acceptance_offer(&plan.revision()?)
                    .await?,
            ))
        }
    }
}
