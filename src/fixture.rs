//! Bounded development tools. This fixture is not a host-shell sandbox or a coding benchmark.
use anyhow::{Context, Result, ensure};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use crate::{
    controller::{Observation, ToolExecutor},
    journal::{ActionIntent, ActionState, Grant, ToolCall},
};

const MANIFEST: &str =
    "shuttle-development-fixture-v1\ncheck: value.txt must contain exactly 42 followed by LF\n";
const MAX_FILE_BYTES: usize = 16_384;

pub struct Fixture {
    root: PathBuf,
}

impl Fixture {
    pub fn open_or_create(root: &Path) -> Result<Self> {
        if !root.exists() {
            fs::create_dir_all(root)?;
            // create_new also protects an interrupted/concurrent initialization from overwriting data.
            write_new(&root.join("fixture.manifest"), MANIFEST.as_bytes())?;
            write_new(&root.join("value.txt"), b"41\n")?;
        }
        let fixture = Self {
            root: root.canonicalize()?,
        };
        fixture.read()?;
        Ok(fixture)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn checked_path(&self, name: &str) -> Result<PathBuf> {
        let path = self.root.join(name);
        ensure!(
            !fs::symlink_metadata(&path)?.file_type().is_symlink(),
            "fixture symlinks are not allowed"
        );
        ensure!(
            path.canonicalize()? == path && path.is_file(),
            "fixture file identity changed"
        );
        Ok(path)
    }

    fn read(&self) -> Result<Vec<u8>> {
        let manifest = bounded_read(&self.checked_path("fixture.manifest")?)?;
        ensure!(
            manifest == MANIFEST.as_bytes(),
            "fixture verification definition changed"
        );
        bounded_read(&self.checked_path("value.txt")?)
    }

    fn hash(contents: &[u8]) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(MANIFEST.as_bytes());
        hasher.update(&[0]);
        hasher.update(contents);
        hasher.finalize().to_hex().to_string()
    }
}

impl ToolExecutor for Fixture {
    fn input_hash(&self) -> Result<String> {
        Ok(Self::hash(&self.read()?))
    }

    fn validate(&self, intent: &ActionIntent, current: &Grant) -> Result<()> {
        ensure!(
            !matches!(intent.call, ToolCall::RunProcess(_)),
            "fixture cannot execute processes"
        );
        ensure!(
            &intent.grant == current,
            "permission changed; unstarted grant is invalid"
        );
        ensure!(
            self.input_hash()? == intent.input_hash,
            "input precondition changed; action was not dispatched"
        );
        if let ToolCall::ReplaceFixture {
            expected_hash,
            contents,
        } = &intent.call
        {
            ensure!(
                current.fixture_writes,
                "fixture writes require a current permission grant"
            );
            ensure!(
                expected_hash == &intent.input_hash,
                "patch expected hash does not match intent"
            );
            ensure!(
                contents.len() <= MAX_FILE_BYTES,
                "fixture edit exceeds file limit"
            );
        }
        Ok(())
    }

    fn execute(&mut self, intent: &ActionIntent) -> Result<Observation> {
        // Recheck immediately before accessing the target. This detects ordinary changes;
        // it does not claim atomic isolation from an uncooperative external writer.
        ensure!(
            self.input_hash()? == intent.input_hash,
            "source changed at execution boundary"
        );
        let (output, check_passed) = match &intent.call {
            ToolCall::RunProcess(_) => anyhow::bail!("fixture cannot execute processes"),
            ToolCall::WriteWorkspaceFiles { .. } => {
                anyhow::bail!("fixture cannot write an admitted workspace")
            }
            ToolCall::PatchWorkspaceFiles { .. } => {
                anyhow::bail!("fixture cannot write an admitted workspace")
            }
            ToolCall::ReadFixture => (
                String::from_utf8(self.read()?).context("fixture must be UTF-8")?,
                None,
            ),
            ToolCall::CheckFixture => {
                let passed = self.read()? == b"42\n";
                (
                    format!(
                        "Development content check: {} (value.txt must be exactly 42 + LF)",
                        if passed { "PASS" } else { "FAIL" }
                    ),
                    Some(passed),
                )
            }
            ToolCall::ReplaceFixture { contents, .. } => {
                let path = self.checked_path("value.txt")?;
                let mut file = OpenOptions::new().read(true).write(true).open(path)?;
                let mut before = Vec::new();
                (&mut file)
                    .take((MAX_FILE_BYTES + 1) as u64)
                    .read_to_end(&mut before)?;
                ensure!(
                    Self::hash(&before) == intent.input_hash,
                    "patch source changed before write"
                );
                file.seek(SeekFrom::Start(0))?;
                file.write_all(contents.as_bytes())?;
                file.set_len(contents.len() as u64)?;
                file.sync_all()?;
                (
                    format!("Replaced value.txt ({} bytes)", contents.len()),
                    None,
                )
            }
        };
        Ok(Observation {
            state: ActionState::Succeeded,
            output,
            input_after_hash: self.input_hash()?,
            check_passed,
        })
    }
}

fn bounded_read(path: &Path) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    fs::File::open(path)?
        .take((MAX_FILE_BYTES + 1) as u64)
        .read_to_end(&mut data)?;
    ensure!(data.len() <= MAX_FILE_BYTES, "fixture file exceeds limit");
    Ok(data)
}

fn write_new(path: &Path, contents: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(contents)?;
    file.sync_all()?;
    Ok(())
}
