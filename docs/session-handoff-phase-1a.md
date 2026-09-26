# Shuttle implementation handoff — phase 1a complete

**Prepared:** 2026-09-13  
**Repository:** `C:\den\CortexShuttle`  
**Current boundary:** phase 1a is complete. Do **not** begin phase 1b until the user explicitly authorizes the durable write-permission increment and selects the requested model/effort. The recommended setting for phase 1b is **Terra: high**.

## Scope and non-goals carried forward

The user asked for a trustworthy local task harness. The implementation already includes a durable process executor, verification plans and receipts, source snapshots, admitted tasks, acceptance/finalization records, and a terminal UI. The recent increment adds a strictly read-only model planning step for an admitted task.

Keep these constraints unless the user changes them:

- Do not create worktrees or commits.
- Do not add llama.cpp/model integration beyond the existing local adapter.
- Do not implement general file writes, acceptance, or finalization as part of the phase-1a planning path.
- Reuse the existing durable request journal and process executor. Do not introduce parallel execution or model-request paths.
- Preserve recovery semantics: save intent before dispatch, mark started before external effects, save result/artifacts atomically, and treat interrupted completion as unknown and non-replayable.
- Preserve input/snapshot binding: a plan or result may describe only the source state it was captured from. A changed input makes the old record stale; it never passes for the new state.

## What phase 1a delivers

Phase 1a is a bounded, read-only admitted-task planning workflow. It lets Shuttle ask the existing local model to produce a small, reviewable proposal after an intake has been admitted. It is intentionally unable to make changes.

The planning context contains only:

- the saved admission and its current source snapshot;
- declared source/test/config/manifest files from that snapshot;
- bounded UTF-8 previews and hashes for those files;
- task objective and constraints;
- verification-plan revision, named checks, and waiver metadata; and
- explicit limitations.

The model is constrained to one typed `record_task_plan` output with:

- a summary;
- workspace-relative paths that could require a later, separate write permission; and
- material limitations.

It has no tool for file reads beyond supplied context, process execution, writing files, granting permissions, accepting work, or finalizing a task. A normal fixture/controller workflow rejects this decision type so it cannot be applied as an ordinary tool action.

## Durable behavior and failure rules

The phase-1a path deliberately uses the existing `model_requests` request ledger and its artifacts. The key guarantees are:

1. A planning request is prepared durably before local-model dispatch and marked started before the call can produce an external effect.
2. The saved request binds the provider profile identity, serialized request, planning-context identity, snapshot identity, and grant.
3. A response is saved through the existing bounded artifact/result transaction.
4. Saving the successful typed proposal and marking the response applied occur in one transaction. A failed proposal insertion leaves the saved response unapplied, so recovery can apply it without asking the model again.
5. A process interruption after the request begins recovers as `unknown`; it blocks replay and does not manufacture a proposal.
6. Inputs are recaptured after the model response. If a declared input changed during planning, Shuttle discards the response and pauses instead of recording a proposal.
7. Re-admission marks associated planning contexts and proposals stale. A new admission requires a new context and a new planning request.
8. Re-running `task-plan` against an unchanged, saved proposal returns that same durable proposal without another inference.

The phase currently requires an action-free admitted run before planning. This is intentional: planning precedes any later write-permission or command-dispatch capability.

## Implementation map

| Area | Main files | Responsibility |
| --- | --- | --- |
| Schema | `migrations/0018_admitted_task_planning.sql` | Immutable planning contexts/proposals, bounded JSON, and monotonic stale status. |
| Workspace workflow | `src/workspace.rs` | Captures bounded declared-file context; binds planning to an admission/run; drives durable request lifecycle; stores/reloads proposal; retires records on readmission. |
| Local model adapter | `src/llama.rs`, `src/llama/stream.rs` | Separate admitted-planning protocol, sole `record_task_plan` tool schema, bounded typed decoding. |
| General request safety | `src/model.rs`, `src/controller.rs`, `src/requests.rs` | Introduces `Decision::AdmittedPlan` and rejects it outside the admitted-task workflow. |
| CLI | `src/main.rs` | Adds `task-plan`; prints a compact receipt-like summary without echoing source previews. |
| Tests | `tests/task_planning.rs` | Covers replay, changed inputs, rollback, readmission staleness, and unknown recovery. |
| Design/status docs | `docs/llama-adapter.md`, `docs/decisions.md`, `docs/implementation-status.md` | Records guarantees, remaining limitations, and ADR S027. |

## CLI and live qualification record

The command is:

```powershell
cargo run --locked -- task-plan --state-dir <state-dir> --profile <local-model-profile.json>
```

For the live phase-1a qualification, the existing user-operated 8080 worker was used. Port 8081 was not needed.

- Profile: `.shuttle/live-repair-worker.json`
- State directory: `.shuttle/admitted-planning-live-01`
- Final admission ID: `74a9e573-f783-4393-acf9-db7ea6ea3e06`
- Final snapshot ID: `4d309540cd69549f51e48357382018198938aad172e0db4ebd3cabcf840aa16a`
- Context ID: `a91093f92c0653312221067c93befbf6fe53fbe5c4c4d6fcc2e72df9b746a283`
- Saved request ID: `d8732c2a-9c6f-44e6-a745-121f83e193d9/request/1`
- Declared files: 43

The worker produced a saved, read-only proposal referring to `src/acceptance.rs`. A second identical `task-plan` invocation returned the same request ID and proposal, proving replay used the durable record rather than making another model call.

The live plan file `.shuttle/admitted-planning-live-plan.json` contains the explicitly declared planning inputs and a waived locked-test check. It is evidence/state for this qualification, not application source.

## Tests and qualification completed

Focused phase-1a coverage in `tests/task_planning.rs` passed:

- planning persists and replays without a second provider call;
- a changed declared input prevents request dispatch and context size remains bounded;
- proposal/result rollback cannot leave a partial receipt or consume the saved result;
- re-admission stales the old context/proposal and requires a new request; and
- an interrupted started request becomes unknown and blocks replay.

Final qualification passed on both platforms:

| Environment | Checks |
| --- | --- |
| Windows | `cargo fmt --all -- --check`; `cargo test --locked`; `cargo clippy --locked --all-targets -- -D warnings` |
| Docker Linux | `cargo fmt --all -- --check`; `cargo test --locked`; `cargo clippy --locked --all-targets -- -D warnings` |

The first final Windows parallel test rebuild exhausted the host paging file while Rust was mapping standard-library test metadata. This was a machine resource condition, not a test failure. After `cargo clean`, the full locked suite passed with `CARGO_BUILD_JOBS=1`, followed by formatting and warnings-denied lint with the same low-memory setting. The Docker qualification completed normally.

## Existing broader guarantees to preserve

The previously completed verification and lifecycle work remains in force:

- versioned source snapshots cover declared relevant source, test definitions, runner configuration, manifests/lockfiles, exclusions, and Git dirty/staged identity where available;
- verification receipts bind check ID, plan revision, pre/post snapshots, executor action/result/artifacts, runtime identity, observed status, and limitations;
- changed inputs before, during, or after verification stale the associated receipts;
- verification executor recovery maintains durable intent, started-before-effects, immutable result/outbox transaction, and unknown-completion replay blocking;
- symlink/reparse paths and unbounded snapshot/output artifacts are rejected or bounded; and
- acceptance/finalization stays explicit and durable, never inferred from a planning or verification observation.

See `docs/implementation-status.md` for the broader sequence and remaining gates, `docs/decisions.md` for the architectural rationale, and `docs/llama-adapter.md` for the local-model request protocol.

## Phase 1b boundary — do not cross yet

Phase 1b is the first capability that could cause a repository modification. It should introduce an **explicit, durable, user-visible write permission** for one narrowly defined proposal, while continuing to use the existing journal and verification evidence.

Before implementing it, agree on the exact permission record and its expiry/revocation behavior. The likely shape is:

1. select one saved, non-stale phase-1a proposal;
2. present the exact proposed paths and limitations for human review;
3. persist a single explicit permission bound to the admission, planning context, proposal, snapshot, and allowed paths;
4. revalidate freshness immediately before any edit intent; and
5. retain unknown/rollback/staleness behavior across restart.

Do not let a model response itself grant write permission. Do not widen file access from the proposal text. Do not write or run a command in phase 1b until this permission contract and its tests are in place.

Recommended model setting for the phase-1b design and crash-boundary work: **Terra: high**. The 8080 local worker may be used only for a later, explicitly authorized live qualification; no model call is required to begin the permission-record design.

## Agreed plan to the first usable state

The first usable state is: give Shuttle a repository task with an approved
workspace and verification plan, let it make bounded changes under supervision,
inspect the evidence, and explicitly accept or reject the result from the
terminal. This is the agreed three-phase, seven-chunk plan. It describes future
work; it does not authorize crossing a phase boundary.

| Phase | Chunk | Status | Completion gate |
| --- | --- | --- | --- |
| 1. General task execution | **1a.** Assemble bounded repository context and connect the admitted task to the existing local model request journal. | **Complete** | |
| 1. General task execution | **1b.** Add narrowly scoped file changes with durable intent, permission and input checks, and restart-safe outcomes. | **Next; stop at its model-change/authorization boundary** | |
| 1. General task execution | **1c.** Run the saved verification suite after changes; handle failure, stale evidence, and re-admission without resetting the run. | Pending 1b | **One real, supervised repository task can make a change and produce fresh suite evidence. A crash or unknown completion cannot silently replay a change.** |
| 2. Review and completion | **2a.** Create an acceptance offer bound to the exact task changes and fresh suite evidence. | Pending phase 1 | |
| 2. Review and completion | **2b.** Connect explicit accept/reject and native finalization to the general task, including interrupted delivery recovery. | Pending 2a | **The user can review the actual result, reject it, or accept it once; restart cannot change that decision or duplicate finalization.** |
| 3. End-to-end terminal flow | **3a.** Provide intake, preflight, admission, execution status, and direction in the TUI. | Pending phase 2 | |
| 3. End-to-end terminal flow | **3b.** Present changes, receipts, limitations, and the acceptance decision clearly; run the Windows and Linux interaction review. | Pending 3a | **The complete supervised path works from the terminal without stitching together CLI commands.** |

Use **Terra: medium** for 1a, 1c, 3a, and 3b. Use **Terra: high** for the
file-change and acceptance crash boundaries in 1b and 2a–2b. Astra is a fallback
only if those boundaries reveal an architectural conflict needing a fresh design
review. The local model on 8080 is needed for live execution qualification; 8081
can wait because embeddings are outside this first usable state.

Each chunk finishes with focused recovery tests and updated guarantees. Each phase
finishes with locked Windows and Docker Linux qualification. Stop for user review
after the first real task reaches evidence, again at the acceptance boundary, and
finally for terminal usability. No worktrees, commits, or automatic Experience
claims are needed to reach this target.

### Phase-1b implementation sequence

Once the user authorizes phase 1b, proceed in this order:

1. Map the exact existing admission, planning-context/proposal, receipt, run, and permission schema relationships. Write the new migration without weakening immutable or monotonic-stale triggers.
2. Define a typed write-permission record and a read-only status/view command. It should identify the admission, context, proposal, snapshot, normalized allowed paths, actor decision, timestamps, and stale/revoked reason.
3. Implement the explicit human-facing grant/revoke transition. The proposal must already be saved and fresh; the transition must not call the model, process executor, or filesystem writer.
4. Bind permission freshness to current declared inputs and re-admission. Changed inputs, changed proposal/context, path normalization failures, or an unresolved request/action must block a grant or mark it unusable.
5. Add focused Windows and Linux tests for grant/revoke transaction rollback, restart, stale input, re-admission, unknown planning request, duplicate/conflicting permissions, path escaping, and durable read-only projection.
6. Update the status and decision documents, then run formatting, locked tests, and warnings-denied lint on Windows and Docker Linux. Stop before enabling an edit action; that is phase 1c.

## Suggested first actions in a fresh session

1. Read this handoff plus the three design/status documents listed above.
2. Confirm the user has authorized phase 1b and has selected the requested model/effort.
3. Inspect the existing admission, planning-context, proposal, journal, and permission-related schema before designing the migration.
4. Write focused failure-boundary tests before enabling any edit dispatch.
5. Keep the stage boundary intact: no writes, acceptance, finalization, worktrees, or commits until the new permission record is implemented and qualified.
