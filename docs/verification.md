# Verification evidence contract

Verification is a harness library API on the existing process controller. The
subsequent [acceptance increment](acceptance.md) adds explicit review/finalization
and a thin `verify` CLI on that API. Neither increment adds a model, worktree or
another command runner.
The existing `shuttle process` command continues to report factual process results;
it does not silently promote arbitrary commands into verification checks.

## Plans and preparation

`VerificationPlan` version 1 contains stable check IDs and human-readable names,
the exact `ProcessSpec` for each check, typed declared inputs, exclusions with
reasons, and explicit check waivers with reasons. The specification includes the
absolute executable and its expected BLAKE3 hash, argument vector, relative working
directory, complete environment, and process limits. JSON serialization uses fixed
struct field ordering and ordered environment keys. A domain-separated BLAKE3 hash
of this versioned representation is the immutable revision ID. Declaration order
is significant; edits to names, arguments, inputs, limits or waivers create another
revision. IDs are evidence identities, not signatures or caller authentication.

Save the plan with `Journal::save_verification_plan`. Capture a `SourceSnapshot`
and construct `ProcessExecutor` from its exact sorted file paths. After the usual
controller bootstrap, call `Journal::prepare_verification` with the revision,
check ID, executor and exact process grant, then call `Controller::dispatch`.
Preparation atomically writes the existing process intent, baseline snapshot and
verification binding. A waived check remains in the durable plan and is never
dispatched or counted as passing. Every executed receipt also records the plan's
waivers. Saving another plan cannot rebind an existing action.

## Snapshots

Source snapshot version 1 records the canonical workspace root, plan revision,
input manifest and categories (source, test definitions, runner configuration,
dependency manifests and lockfiles), resolved files with content hashes, byte
counts and platform permissions/attributes, declared exclusions, observed check
executable hashes, and the executing harness binary hash. File contents are hashed,
not copied into an unbounded source archive. Declared directories are inventoried
recursively so newly added source files invalidate prepared or observed evidence.
Waived executables need not be available and are not probed.

For a repository with a local workspace-root `.git` directory, snapshots capture
HEAD, its current loose ref when present, packed refs and the raw index identity,
plus a separate declared working-file identity. These preserve committed/staged
and dirty working-state identities without an additional command path. They do
**not** compute porcelain clean/staged flags, parse the index or Git objects, or
claim coverage of undeclared working files. Index stat refreshes and unrelated
packed-ref changes can conservatively invalidate receipts. Missing Git metadata
is explicitly recorded; indirect Git directories are rejected. Parent repositories,
submodules and alternate/shared Git stores are outside this increment.

Limits are 64 checks, 256 input declarations, 256 resolved files, 1,024 visited
entries, 32 directory components, 16 MiB total source bytes and 16 MiB Git metadata
bytes per capture. Each serialized plan, source snapshot and receipt is at most
64 KiB; process output retains the executor's existing limits. Snapshots contain
hashes rather than raw source or index bytes. Cumulative journal retention remains
unbounded. Missing/unreadable inputs or exceeded bounds fail closed.

All ordinary source, executable, Git metadata and workspace symlinks and Windows
reparse paths are rejected. Encountered excluded entries are checked before being
skipped; exclusions do not authorize following a link. Traversal and non-UTF-8
manifest paths are rejected. This is not protection against hostile filesystem
races, hard-link aliases, or exotic filesystem semantics.

## Receipts and freshness

Before dispatch and again after the durable started marker, capture and compare
the current snapshot with the prepared snapshot. The process executor still
rechecks its own exact grant, executable and input preconditions at launch. After
the process tree is observed stopped, capture the post snapshot. Commit that
snapshot, the immutable verification receipt, process result and output artifact,
time accounting and native outbox in the existing completion transaction. Failure
at any insertion rolls back the whole completion. A post-snapshot failure leaves
the action uncertain rather than inventing a passing receipt.

Receipt version 1 binds the check ID and plan revision, pre/post snapshot IDs,
process action ID, immutable result and artifact hash, exact process specification,
harness version/binary hash, operating system/architecture, waivers and limitations.
Observed process success means only that the named command exited zero. A changed
pre/post snapshot or a changed executor-observed input hash creates a stale receipt
even if the process succeeded or the files reverted before snapshot capture. The
historical action result remains unchanged. A nonzero exit or cancellation never
passes. Generic process actions still have no verification receipt.

`verification_receipt` refreshes freshness before returning evidence.
`offer_verification_receipt` additionally requires the exact requested revision;
evidence from another revision is rejected. Restart, dispatch and post-execution
also refresh earlier receipts. Snapshot failure or any observed relevant change
durably marks affected receipts stale. Staleness is separate from the immutable
observation and monotonic: restoring files does not revive a stale receipt.
Started actions without a committed result become unknown on restart, keep their
plan/snapshot bindings and time reservation, and block replay. No receipt is
fabricated for an unknown action.

Freshness is checked at these boundaries, not continuously. An offer is valid only
for the instant it was checked. The acceptance consumer revalidates the exact
evidence when the user responds rather than reusing a cached `passed()` value.
There is no watcher or atomic filesystem snapshot. Changes made and restored
wholly between captures may escape
detection. Unlisted dependencies, dynamic libraries, external services, tool/test
semantic completeness and hostile concurrent mutation are not qualified. A custom
Linux supervisor's binary identity is not separately attested. None of these
receipts constitutes user acceptance, task finalization or an Experience claim.

## Admitted suites

An admitted general task can run every unwaived check sequentially with
`task-verify-all`. The admission retains one durable run and an immutable mapping
from each check ID to its action identity. The runner revalidates the admission
before each check. A changed input blocks later checks, an unknown action blocks
replay, and each completed receipt keeps its own stale state. A failed check may
resume the declared suite only after its result is durable and native delivery is
complete. `task-verify-status`
reports waived, pending, not_prepared, passed, failed, stale and unknown checks.
`not_prepared` means the check was bound to the admission but its preparation
was refused, so no action exists and nothing started; `unknown` is kept for a
prepared or started action with no receipt. It opens the
journal, so it may recover interrupted work and persist staleness. It does not
dispatch work.

## Admitted suite evidence

`task-verify-evidence` records a versioned durable readiness claim only when all
unwaived admitted checks have fresh passing receipts for the admission's exact plan
revision and source snapshot. The record binds each check, durable action and
receipt hash and retains declared waivers. It cannot be recorded for failed,
pending, unknown, stale, changed, or waived-only suites. `task-verify-status`
refreshes the record and marks it stale if any required receipt or current declared
input no longer matches. This is reviewable verification evidence only; it does
not constitute acceptance or finalization.

## Re-admission after changed inputs

Inspect the saved revisions with `shuttle task-admission-history --state-dir
<task>`. This command reads history without opening a controller or recovering
interrupted actions. To establish a new baseline, use:

```text
shuttle task-readmit --state-dir <task> --from-admission <current-id> --request-key <unique-key> --reason "Explain the changed inputs"
```

The command retains the immutable intake/plan and the existing run's cumulative
accounting. It records a fresh snapshot and successor admission in one transaction,
and permanently retires the old admission's receipts and suite evidence. No check
executes until separately requested with host-execution approval. Run
`task-verify-all` and `task-verify-evidence` again to qualify the successor.

An identical request key, predecessor and reason returns the saved response;
reusing that key for a different request fails. An unchanged fresh admission
cannot be replaced. Unfinished or unknown work, pending delivery, stalls and
acceptance state must not be bypassed through re-admission. Plan, objective,
workspace and constraints cannot change through this command. The history grows
with each revision; individual admission and snapshot artifacts retain their
existing size bounds. This increment does not add acceptance or finalization.
