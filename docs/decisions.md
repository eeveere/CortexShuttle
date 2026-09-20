# Architecture decisions

## S001 — Application boundary and initial repository

2026-09-03. Use `C:/dev/CortexShuttle` for the separate repository, `Shuttle`
as the product name, `cortex-shuttle` as the development crate, and `shuttle`
as the executable. The native adapter calls CortexWeave application services.
Its database schema and SQL remain entirely inside CortexWeave.

The controller, journal, tools, model provider, and terminal are separate modules.
This slice supplies a deterministic model driver and bounded fixture tools.
It does not select or launch a live worker or embedding service.

## S002 — Durable action and delivery boundaries

2026-09-03. Store harness state in SQLite using WAL and synchronous FULL. Store
bounded output bytes as content-addressed artifacts in the same database; commit
the observed result, artifact and ordered outbox in one transaction. A dedicated
OS file lock prevents two cooperating controllers from owning one journal.
Resolve existing database paths before selecting the lock path. This mechanism
assumes an ordinary local state directory; aliases through hard links, hostile
filesystem writers, and network filesystem lock semantics are not qualified.

Before a tool effect, commit the exact intent and then the started marker.
Reopening converts started actions without durable results to unknown and pauses
the run. Same-ID submissions compare immutable intent and return existing results.
Prepared actions require current permission and input revalidation. Unknown
actions cannot be automatically executed under their original ID.

Creation requests also enter the outbox before delivery. CortexWeave resolves
identical request keys to immutable original receipts. A request conflict remains
an error; the harness does not invent another identity to work around it.

## S003 — Honest development fixture and preview

2026-09-03. The initial fixture uses a fixed manifest and one small text file.
Its four actions are read, failing content check, exact replacement and passing
content check. The input identity covers both content and the check definition.
The content check is explicitly not a compiler/test-runner verification contract.
Factual tool-result Events are delivered; episode membership and consolidation
belong to the next complete coding experiment.

The journal ends this run at `awaiting_review`. No model response or development
check manufactures actual user acceptance. The separate Ratatui preview shows
sample interactions, including acceptance, without writing any of them into the
real task history. It needs human review and native terminal qualification.

## S004 — Substrate integration pin

2026-09-03, updated 2026-09-06. CortexWeave v0.5.1 at commit
`fe60a2334be774d0bdac67d4243d604788571eb4` is the qualified substrate revision.
It includes the native ingress receipt extension, verified against its native
and historical-evidence contracts. Shuttle uses Cargo's Git revision dependency
rather than a local path, so its lockfile resolves the same substrate source.

Before a supported live experiment, capture the actual worker/server/template
configuration. Product release and live qualification remain separate from this
initial foundation.

## S005 — Host execution and conservative recovery

2026-09-07. Real commands share the existing durable intent, started marker,
result/artifact and native outbox transactions. Grants identify the exact command,
canonical workspace root and declared input manifest. Reopening prepared work
revalidates the grant, input bytes and executable hash. Missing post-run identity
or incomplete process cleanup leaves completion uncertain and blocks replay.

Windows 10+ assigns a private kill-on-close Job Object at process creation and
restricts handle inheritance to its three standard streams. Linux uses a separate
Shuttle subreaper process with a private lifetime pipe; it reaps and terminates
adopted descendants after cancellation or parent death. These are process-lifetime
mechanisms, not filesystem/network sandboxes. Neither terminal access nor ambient
environment is inherited by a command. Unsupported platforms reject dispatch.

Reserve each command's entire timeout plus cleanup allowance atomically with the
started marker. Refund only against an observed result in the completion
transaction. Unknown work retains the reservation across restart. Process failures,
cancellation and three identical successful observations pause durably; provenance
delivery cannot undo that pause. S008 below records the subsequent
model-time/no-progress/replan controller increment.

Keep raw output bounded and count discarded bytes; do not infer verifier success
from exit status. Native Events carry a compact factual summary and the journal
artifact hash. See [the execution contract](process-execution.md).

## S006 — Versioned verification plans, snapshots and durable receipts

2026-09-07. Keep verification evidence in the harness journal. Version 1 plans
use domain-separated content revision IDs and declare named checks, exact process
specifications, typed source/test/configuration/dependency inputs, exclusions and
reasoned waivers. Bind an action to one immutable revision and baseline snapshot
in the same preparation transaction. Use the existing process executor/controller
for dispatch and the existing result/artifact/outbox transaction for completion;
there is no parallel verification command path.

Bound snapshot inventories and bytes. Hash declared files and executable/runtime
identity before and after execution, including directory membership and platform
permissions. Preserve Git HEAD/ref/index and declared working-file identities when
local metadata is available. This identifies staged and dirty input states without
pretending to provide porcelain classification or complete Git workspace coverage.

Keep historical observations immutable and store monotonic stale status separately.
Changed or unavailable current input identity cannot pass; stale evidence cannot
revive when files revert. Revalidate on dispatch, after execution, journal reopen
and receipt offer. Offers require the exact plan revision. Missing completion or
post-run identity preserves unknown recovery and blocks replay. Waived checks stay
explicitly waived; they never manufacture passing execution receipts.

This increment provides bounded sampled evidence, not an atomic snapshot, watcher,
OS sandbox, full runtime attestation, test-suite completeness claim or user
acceptance. The detailed API, verified boundaries and limitations are in
[the verification contract](verification.md). Model integration and acceptance /
finalization remain separate gates.

## S007 — Explicit acceptance and ordered recoverable finalization

2026-09-07. Treat acceptance as immutable input from a trusted user interface.
An offer binds one exact plan, current snapshot, latest unwaived check receipts,
waivers, limitations and action history. Recheck those identities on response,
persist rejection/acceptance explicitly, and keep stale offers monotonic. An active
offer ID prevents an old response from changing a newer offer. No model/tool enum
can accept, and no passing process implicitly accepts.

Commit acceptance and all finalization intents atomically in the existing outbox.
Deliver the user Event, ordered episode membership, episode closure, task completion
and session closure in order. Seal accepted runs; finalize only with the last
durable acknowledgement. Retry deliveries without reexecuting commands or extending
acceptance to later file states.

Keep the qualified CortexWeave pin. Event/episode APIs support request-key recovery.
Older task/session terminal APIs require typed state reconciliation: reuse only
exact accepted task details and owned terminal session state, and block conflicts.
Writes remain application-service calls; public typed storage reads do not expose
native SQL/schema to Shuttle. This assumes one cooperating lifecycle writer and
is weaker than receipt-backed terminal APIs under concurrent external mutation.

The thin verify/review/respond/finalize CLI shares the executor and outbox. Recording
a response requires typing the offer ID at an interactive terminal. See the
[acceptance contract](acceptance.md) for guarantees and remaining filesystem,
native-lifecycle and user-interface limitations.

## S008 — Account for attempts and active work before they begin

2026-09-07. Persist each provider request's exact typed context, provider identity,
permission and timeout before invoking it; reserve one of 64 request slots in the
same transaction. Persist started separately, then an immutable response/error
before applying a proposal. Saved responses prepare their exact action atomically
with application, and never trigger another provider call on recovery. Unobserved
started requests become unknown and block new work. Invalid or stale proposals
are durably discarded. Missing token usage remains unknown rather than zero.

Measure controller work with a monotonic clock. Reserve active time durably before
work and settle only on observed return; interrupted work retains the reservation.
Nested executor/snapshot/delivery work shares a span. Native delivery and user review
remain possible through charged maintenance spans after exhaustion, preserving
accepted finalization. The process-only budget and its atomic result settlement
remain separate and unchanged. Pre-migration time is only a process-time lower
bound; migrated request counts are explicit placeholders with unknown observations.

Commit progress fingerprints with action results. Pause on six actions or five
active minutes without new observed evidence, retaining the earlier exact process
repetition rule. Allow one caller-directed resumption with an idempotent key; keep
total budgets and known evidence. The next request records the supplied replan
direction. Neither provider output nor a native acknowledgement can authorize it.

These are bounded execution and observed-evidence policies, not semantic progress
proof, token billing, a hard real-time watchdog or a live-model qualification.
See [run accounting](run-accounting.md) for recovery boundaries and limitations.

## S009 — Commit native terminal receipts with their effects

2026-09-07. Replace the S007 adapter's read/reconcile window with CortexWeave's
request-keyed task/session terminal operations. The substrate owns the immediate
SQLite transaction containing prior-state checks, mutation and immutable receipt.
Session closure links the exact task receipt and owner marker. Concurrent retries
return the original receipt; conflicting writers cannot claim or overwrite a
terminal effect. Existing outbox identities and acknowledgement transactions stay
unchanged. A legacy terminal effect without a receipt cannot be attributed safely
and leaves recovery pending for inspection. This supersedes S007's assumption of
one cooperating terminal writer, but is not a session-admission lock or database
authorization mechanism.

Keep the Git baseline in Cargo metadata and use an explicit sibling path patch
while both repositories remain uncommitted. Locked builds currently require that
patched sibling; the path is not an immutable revision. Publishing/pinning a new
qualified native revision remains a separate change. No native SQL is copied into
Shuttle and no alternate executor or outbox is introduced.

## S010 — Qualify terminal ownership separately from human usability

2026-09-07. One scoped terminal owner performs setup and restoration across normal
exit, errors and unwinding, including exact Windows console modes. Extract bounded
input handling so key semantics can be checked independently, then exercise actual
UI code through ConPTY and Linux PTY children. Assert rendered resize dimensions
and mode restoration; terminal byte streams can omit unchanged glyphs.

Enable bracketed paste only for the Unix reader that decodes it. Document Windows
ordinary paste and terminal-dependent newline limitations. Automated interaction
and buffer rendering do not establish human visual/usability sign-off. See
[terminal qualification](terminal-qualification.md) for the review procedure,
ownership assumptions and forced-termination limits.

## S011 — Replace the development path override with the published receipt revision

2026-09-08. The user published CortexWeave's terminal receipt patch at
`754126bfc2efd6330253826c89922148d33d9915`. Verify that identity against the remote,
pin it in Shuttle's dependency declaration and lockfile, and remove the sibling
path override introduced in S009. Retain the earlier baseline in metadata only.
The lockfile change is limited to CortexWeave's source identity; unrelated package
versions stay unchanged. Linux qualification mounts only Shuttle's source, so a
local native checkout cannot satisfy the dependency accidentally. This completes
the dependency publication/pinning gate; it does not authorize model integration
or record acceptance of a harness task. Shuttle remains uncommitted.

## S012 — Bind local inference to exact durable requests and bounded transport evidence

2026-09-08. Following authorization for the adapter increment, extend the existing
provider boundary with pure request preparation and a prepared-response entry point.
Persist the exact body, tool schemas, profile and planned exchange sequence before
the shared started marker. Freeze the profile across the run. Preserve old JSON
records through optional fields; no parallel ledger or command executor is added.

Permit one inference POST per attempt, with declared pre/post metadata observations.
Disable automatic retries, redirects and proxies; restrict this increment to numeric
loopback HTTP and explicit `autoload=false`. The caller owns service lifecycle.
Commit bounded raw exchange artifacts and immutable result together, retaining
unknown-completion replay blocking and saved-result recovery. Validate complete SSE
and strict single-tool arguments before the existing executor can apply a proposal.

Capture executable, weights and server/template/configuration identity, while
explicitly limiting the claim to sampled metadata and local files. This does not
attest loaded memory, runtime libraries or GPU residency. Qualify read-only live
transport in both thinking modes; factual producers and consolidation remain gates
before the first complete live repair. See [the adapter contract](llama-adapter.md).

## S013 — Qualify saved producer observations and freeze consolidation dispositions

2026-09-08. Derive compiler/test evidence only from complete, fresh verification
receipts and their immutable process artifacts. Bind normalized evidence to the
exact plan, snapshots and runtime record; commit qualification and native delivery
intent together. Keep the original result unchanged and preserve stale status.
Use a narrow rustc JSON profile and the published unittest capture helper, with
independent harness snapshots and the native eligibility registry. Do not duplicate
native normalization rules or treat arbitrary exit zero as qualified test evidence.

Insert consolidation into new acceptance finalizations after episode closure and
before task completion. Save the native preview before automatic acceptance, use
the exact proposal identity for retry, and save the disposition Event before native
delivery. Review-required and typed no-result are valid recorded outcomes. Keep
already committed legacy finalization sequences intact. Count qualified evidence
alongside action Events for the episode limit. No producer calls the model or adds
a command path; no model can invoke user acceptance. See
[producer qualification](evidence-qualification.md) for verified scope and limits.

## S014 — Gate a live fixture repair on real baseline and final evidence

2026-09-08. Persist a versioned live workflow configuration and both exact process
plans before inference. Reuse the shared executor, journal and native outbox. Allow
only a qualified expected baseline assertion failure to continue into model work,
without resetting accounting. Give baseline and final capture outputs distinct
action-derived paths. Validate fixed test/runner definitions before model effects.
Keep a stable native executor mode across stages and box nested workflow futures
to fit the Windows CLI stack.

Present completed fixture actions compactly under a new provider protocol identity;
retain complete context and exact wire artifacts durably. Distinguish successful
tool execution from a passing check. Preserve failed/stalled trials and the single
recorded operator replan; do not turn another trial into hidden replay. A fresh
qualified final check creates an offer, with actual user acceptance still separate.
See [qualified live repair](live-repair.md) for the contract and limitations.

## S015 — Transport fixture replacement content as explicit UTF-8 bytes

2026-09-09. The saved v3 HTTP responses already contained literal backslash/n or
omitted LF, and their decoded contents matched the journal exactly. Do not repair
such proposals by appending newlines or interpreting escapes a second time. The
running build's parser source also lacks the historical unconditional whitespace
trim, so that old upstream issue is not established as the cause of these trials.

Protocol `shuttle-llama-fixture-v5` replaces the provider's string argument with
`utf8_bytes`, an array of integers from 0 to 255. Strictly decode valid UTF-8 within
the existing 16,384-byte replacement bound, then pass the exact text to the same
fixture executor. Reject the legacy argument under this new identity. Reconstruct
saved replacement calls with byte arrays and expose bounded read observations as
bytes as well as text. Keep the original request/result/artifact transactions,
grants, hash preconditions and replay rules. No journal migration or service change
is required; older provider identities cannot silently resume under v5.

The initial v4 trial received HTTP 400 before inference: `maxItems: 16384` produced
a grammar repetition beyond the worker's supported limit. V5 omits that schema
keyword while retaining the decoder size bound and transport bounds. Preserve the
failed trial instead of retrying or changing its durable request. The supplied
worker console log corroborates the saved HTTP error.

## S016 — Separate observation from controller ownership and bind terminal starts

2026-09-09. Live workspace polling must use a read-only SQLite connection and one
transaction per projection. Reusing `Journal::open` for observation acquires the
exclusive controller lock and runs recovery, preventing another controller from
advancing the task. Observers neither migrate nor recover; STARTED remains
STARTED until an actual controller owner performs recovery. Saved staleness is
displayed honestly; explicit review retains the existing freshness checks.

Migration 0008 records an immutable workflow binding for newly created scripted
tasks. Creation initializes only a fresh isolated task; an explicit two-press
terminal confirmation authorizes its declared fixture scope. Both terminal and
demo CLI call the same existing controller, model ledger, fixture executor and
native outbox. Recheck run/workflow/recovery state under exclusive ownership.
Unknown work blocks start; older or model-backed tasks are never inferred to be
scripted. Changed/unavailable observer state cancels pending confirmations.

Task selection observes immediate saved directories; no scheduler, alternate
command path, arbitrary code-generation workflow or acceptance button is added.
Terminal mode is restored before the selected workflow runs. Cross-database task
initialization is not atomic; incomplete directories survive for inspection and
are never overwritten. See terminal qualification for verified scope and limits.

## S017 — Persist an immutable intake before admitting a general task

2026-09-09. A general task needs a durable description before it can be connected
to any controller, model, process executor, or acceptance workflow. Store one
bounded, versioned intake in the existing journal database: canonical existing
workspace root, objective, constraints, full validated verification plan and its
stable revision. The row is append-only in practice through a singleton immutable
table and update/delete triggers. Reuse the read-only task observer so inspection
does not own the controller lock, migrate state, recover work, or alter receipts.

An intake is descriptive and explicitly non-runnable. It creates no `runs` row,
grant, process intent, snapshot or receipt. Existing scripted fixtures cannot be
adopted, and the scripted controller rejects an intake. Any future general
admission must independently revalidate the stored plan and capture fresh source
snapshots before dispatch; saving a revision is not a filesystem watch, a source
snapshot, permission, model integration, or authorization to execute commands.
Reject malformed/oversized plans, nonexisting workspaces, unsafe terminal controls
and attempts to reuse an existing task directory rather than overwriting state.

## S018 — Record preflight snapshots without admitting execution

2026-09-09. A saved intake needs reviewable current-state evidence before a future
workflow can be considered, but snapshot capture must not silently become task
admission. `task-preflight` opens the existing journal exclusively only for its
atomic durable write, validates the immutable intake identity/revision/workspace,
then captures the existing bounded declared-input snapshot. In one transaction it
saves the plan, snapshot and an immutable intake-preflight link. Distinct source
states remain separately recorded; a recapture cannot overwrite an earlier state.

No `runs` row, action intent, grant, executor, process, provider request, receipt,
offer or finalization is created. The command rejects admitted scripted tasks and
does not attempt recovery/replay of a general workflow. A later admission must
define its own explicit controller and authorization contract and recapture its
pre-dispatch evidence; preflight is not a watcher, approval, or permission grant.

## S019 — Make superseded intake preflights durably stale

2026-09-12. Intake evidence must not silently become evidence for a changed
workspace. On a successful new preflight capture, mark older preflight bindings
for that same immutable intake stale in the transaction that stores the new plan
and snapshot. Keep source snapshots and bindings immutable; only the nullable
stale reason may transition once. A later file restoration cannot revive stale
evidence. The observer reports the latest snapshot and the retained stale-history
count without taking ownership or capturing files itself.

This remains a pre-admission evidence boundary. It does not continuously watch
the filesystem, detect changes until a preflight refresh occurs, authorize a
workflow, or create an executable action/receipt.

## S020 — Bind general-workflow admission to a fresh preflight

2026-09-12. Saving a task description and source observation is insufficient to
begin a future workflow unless their relationship is explicit and current.
`task-admit` re-captures the stored plan's declared inputs under journal ownership
and requires an exact match with the latest non-stale preflight before writing one
immutable `general_verification_v1` admission record. The record binds intake,
workspace, plan revision and snapshot, and prevents later preflight replacement.

Admission is a contract boundary, not execution. It creates no run, action,
grant, executor, process, provider request, receipt, offer or finalization. A
later controller must consume this exact contract and independently preserve the
existing durable intent/unknown-completion rules.

## S021 — Validate admission evidence before constructing a controller

2026-09-12. Future general execution must consume one checked admission contract,
not independently reinterpret task files. The controller-facing reader loads the
immutable admission under journal ownership, verifies its intake/plan/snapshot
identity, rejects stale or missing evidence, and compares a fresh declared-input
capture to the admitted snapshot. It returns no executor and creates no run or
effect; source changes block the next layer before dispatch can be considered.

## S022 — Dispatch admitted checks only from saved contract state

2026-09-12. The first general dispatch entrypoint receives no workspace or plan
from its caller. It loads the admission-owned plan and workspace, rejects waived
checks and changed inputs, then reuses the existing durable verification executor.
The original intent/start/result/outbox/receipt recovery rules remain the source
of process safety. Focused admitted-command coverage and final Windows/Docker
Linux qualification confirm the saved-contract entrypoint uses that path; it does
not create a second executor or relax stale and unknown-completion handling.

## S023 — Bind the first admitted run to one check and action identity

2026-09-12. The first general verification slice permits one admitted run/check/
action identity. Persist that mapping before preparing the shared verification
intent. Recovery may revalidate and observe that exact action, but a changed check
or action ID conflicts rather than dispatching new work. This keeps the existing
unknown-completion and immutable result rules intact while the multi-check workflow
is still out of scope. The final locked Windows and Docker Linux suites passed with
the binding in place. This does not add orchestration for multiple checks, model
integration, user acceptance/finalization, worktrees, or commits.

## S024 — Record every admitted check before sequential suite dispatch

2026-09-12. One general admission owns one durable run and an immutable ledger of
named check/action identities. Preserve the original first-check binding for
migration compatibility, then add new check bindings without replacing any saved
identity. `task-verify-all` derives one stable action identity per admission/check
and invokes the existing verification preparation, dispatch, result and receipt
path in declared-plan order. It reloads the admission before each dispatch, so a
changed workspace blocks the remainder of the suite. Failed checks retain their
observed result; the suite can resume only from that durable failed verification
state with no pending native delivery. Unknown actions block replay; stale receipts
remain stale.

The read-only suite status projection reports waived, pending, passed, failed,
stale and unknown entries without authorizing a process. This decision does not
add parallel execution, retry policy, model integration, user acceptance or task
finalization.

## S025 — Make admitted suite readiness a durable evidence gate

2026-09-12. A list of individually passing receipts is not itself a reviewable
claim that one admitted plan is currently complete. Persist a single immutable
suite-evidence record only after every unwaived check has a fresh passing receipt
for the admitted plan revision and snapshot. Bind each receipt hash and action ID,
retain explicit waivers, and refresh it whenever status is read. A changed input
or stale/missing/failed receipt marks the record stale permanently. The gate
neither accepts work nor finalizes a task; it produces evidence for a later,
separate user-review boundary.

## S026 — Version admissions within the existing durable run

2026-09-12. Re-admission advances a current-admission pointer to a new immutable
revision, bound to a freshly captured snapshot of the same saved intake and plan.
It retains the original run, objective, budgets, action identities and recovery
history. Creating a replacement run would require broader accounting and recovery
changes and could hide unresolved work; it is unnecessary for this increment.

Migration 0017 retains the original admission bytes and ownership, then introduces
an append-only predecessor chain and an explicit request key and reason. One
transaction saves the snapshot, successor and ownership, retires old receipt and
suite evidence, and advances the pointer. A failed transaction leaves no partial
successor. Retrying an identical request returns the original response, even after
a later revision; changing its predecessor or reason conflicts.

Re-admission requires changed inputs or already stale evidence and settled work.
Prepared/unknown actions, unresolved model work or active-time reservations,
pending native delivery, stalls and acceptance state block it. It neither resets
budgets nor authorizes execution. Current-admission ownership and snapshot checks
also apply inside the shared verification preparation and dispatch path. New
admissions receive new check identities; historical results and waivers remain
immutable, and restored files cannot revive retired evidence. Suite execution
retains any previously saved single-check identity and refreshes all receipts
before reporting aggregate success.

This is bounded requalification of one immutable intake, not plan editing, a
retry policy, unknown-completion resolution, or user acceptance. Snapshots remain
boundary observations, with the filesystem and external-input limitations in
`verification.md`. Admission history is read-only; suite status opens the journal
and may durably recover interrupted work and mark evidence stale.

## S027 — Keep admitted-task planning read-only and in the request ledger

2026-09-13. General-task planning uses the existing local-model request ledger,
with a new fixed planning protocol rather than widening the fixture repair tools.
An admitted snapshot produces a bounded context containing only declared-file
previews, identities, objective, constraints and verification metadata. The model
can return one bounded proposed plan as data; it cannot request a file write, run
a command, grant itself permission, accept work or finalize a task.

The exact serialized request, profile and bounded transport artifacts retain the
normal intent/start/result recovery contract. A proposal becomes visible only when
the successful model response and proposal record commit together. A rollback
leaves the saved response unapplied for retry without another inference call.
Changed inputs discard an in-flight response; restart makes started work unknown.
Re-admission stales historical planning context and proposals permanently. This is
not a general coding agent yet: phase 1b owns explicit write intent and permission.

## S028 — Make write permission explicit, immutable, and independently revocable

2026-09-13. A model proposal remains untrusted data until a human grants one
durable permission for its exact current planning context. Migration 0019 binds a
grant to the admission, planning context, proposal request, admitted snapshot and
normalized workspace-relative proposed paths. The grant records its caller-supplied
idempotency key, actor and timestamp. It neither creates an edit intent nor gives
the model, executor or filesystem a new capability.

There is no clock expiry: the permission remains inspectable until it is revoked
or made stale. Revocation is a separate append-only, irreversible record with its
own actor, key, reason and timestamp; a revoked proposal cannot be re-granted.
Re-admission stales its old permission permanently. A current input mismatch is
reported as unusable and blocks a grant; any later writer must recapture and
revalidate the same admission/snapshot immediately before it saves edit intent.

`task-write-status` uses a read-only journal connection and only projects saved
proposal/permission state plus a fresh input comparison. `task-write-grant`
requires the exact printed context ID and an explicit proposed-path approval flag.
This preserves a visible human decision without treating model text as authority.
The phase intentionally stops before an edit dispatcher, a process launch, test
execution, acceptance or finalization.

## S029 — Bind one model patch to one granted proposal, then re-admit before tests

2026-09-13. Phase 1c uses a second, bounded response from the same saved local
worker profile to obtain exact UTF-8 replacement bytes. The fixed editing protocol
can name only existing declared regular files and must supply each pre-image hash.
The workspace workflow validates that every path is in the immutable grant, every
hash matches the admitted snapshot, and inputs remain fresh immediately before it
saves a journal action and again before it writes.

The model response and prepared write action are committed together. The journal
marks that action started before the first filesystem effect; interruption is
unknown and blocks replay. Writes are bounded and synced, but are not a filesystem
transaction, so an interrupted multi-file patch is intentionally inspectable only,
not retryable. A successful patch changes the admitted snapshot; the operator must
use the existing explicit re-admission flow before `task-verify-all` can produce
fresh suite evidence. Acceptance and finalization remain separate later gates.

## S030 — Offer review evidence without importing generic acceptance finalization

2026-09-13. Phase 2a adds a distinct immutable, review-only offer for an admitted
task. It binds the current admission, objective, exact successful
`WriteWorkspaceFiles` action ID and artifact, full action-history identity, and
the exact fresh suite-evidence record. Offer creation requires a ready run with no
pending/unknown model, action, or native-delivery work; it also re-captures the
declared inputs immediately before persisting the offer. A request key makes the
offer idempotent and conflicting reuse fails.

The existing generic acceptance path cannot safely be reused here: it assumes an
outbox receipt for every action, while the admitted workspace patch intentionally
has no native delivery. Phase 2a therefore creates no user decision, task/session
completion, Experience claim, or finalization intent. `task-offer-review` remains
readable after source drift and monotonically marks the saved offer stale; a later
re-admission also retires it. Phase 2b must introduce the explicit accept/reject
and native finalization bridge as its own audited transition.

## S031 — Finalize an admitted task only after a task-specific explicit decision

2026-09-13. Phase 2b keeps the admitted-task offer separate from the legacy
generic acceptance table because the bounded workspace write has no pre-existing
native result Event. Migration 0021 records one immutable, request-keyed accept
or reject decision for the exact task offer. Acceptance revalidates the current
admission, exact suite evidence, edit action/artifact and full action-history
identity immediately before it commits the decision.

In the same transaction, acceptance creates a factual native Event for the
already-observed patch, a user-acceptance Event, episode membership/closure,
consolidation, task completion and session closure intents, then enters
`finalizing`. The existing ordered outbox, acknowledgement validation, recovery
and `finalized` transition remain authoritative; no filesystem edit or verification
is replayed. Rejection records only the durable user choice and creates no native
finalization work. An accepted task is sealed, including against re-admission.

`task-offer-respond` is interactive and requires typing `accept <offer-id>` or
`reject <offer-id>` after displaying the persisted review. `task-finalize` only
resumes a previously accepted run. These events preserve factual provenance but
do not claim an Experience or an automatic consolidation disposition.

## S032 — Keep the end-to-end terminal as a durable-workflow client

2026-09-13. The `task` terminal projects the saved general-task intake,
preflight/admission state, recovery state, suite evidence, patch/review material,
limitations and explicit decision. Each shortcut requires a second explicit
press, restores the terminal, then calls the same journal-backed workflow used by
the headless CLI. The durable workflow remains the authority for freshness,
permission, unknown completion, idempotency and native delivery recovery.

`shuttle task` can create the initial intake from a declared workspace, objective
and plan, then drive preflight, admission, planning, grant, edit, verification,
offer and accept/reject without manually stitching task subcommands. A model
profile and host-execution approval are explicit launch parameters. The terminal
only renders bounded factual summaries; it cannot manufacture a pass, grant,
acceptance or finalization from display state.

## S033 — Apply bounded exact text hunks under a versioned admitted-edit contract

2026-09-16. Accepted design for the next implementation chunks; **not an
implemented capability**. This supersedes S029's whole-file wire representation
for future admitted edits only. S015's exact-byte principle and S028–S032's human
authority, durability, verification and review gates remain in force. Fixture v5
and admitted planning v1 keep their existing semantics. Terminal lifecycle changes
are outside this decision.

The retained emCP `r4` attempt failed at the 65,536-byte streamed transport bound,
with HTTP 200 but incomplete output and unknown usage, before any workspace
action. Preserve it as failed v1 evidence. The
[frozen projections and provenance](manual-qualifications/emcp-2026-09-16-r4.md)
contain no raw model content. Qualification of v2 requires a fresh task directory;
do not resume, reset budgets, migrate the meaning of, or reinterpret `r4`.

### Protocol, identity and compatibility

Use `shuttle-llama-admitted-editing-v2`, provider identity
`shuttle-llama-admitted-editing-v2:<profile_digest>`, and purpose
`admitted_task_edit_v2:<session_id>:<turn_index>` (zero-based). Persist protocol
version 2 and bounds revision 1 in the session, serialized request and prepared
patch. `profile_digest` is exactly the existing profile identity digest: lowercase
64-character BLAKE3 of `serde_json::to_vec(&LlamaProfile)`, with the existing struct
field order, compact UTF-8 JSON, `serde_json` number/string encoding and recursively
lexically ordered JSON object keys in the saved normalized `properties` value.
No domain prefix, re-normalization or subset of profile fields is used. Freeze
this serialization for profile version 1; changing it requires a new profile
version rather than silently changing the digest. Compare this exact digest in
the provider, session and request. Session identity binds the run, admission,
context, proposal request,
permission, snapshot, profile digest, protocol and bounds revision. One session
and at most one patch action belong to a permission. Exhaustion, invalid output,
revocation or failed/unknown completion closes that session to further inference;
reinvoking edit never replenishes its budget.

The current request ledger pins a profile independently of mode and includes a
legacy prefix compatibility rule (`same_provider_profile`). V2 must explicitly
separate profile equality from protocol equality: allow the same validated profile
to progress from admitted planning v1 to editing v2, but compare the full provider,
purpose, session, turn, serialized request and protocol for request reuse. Parse
only enumerated provider forms; equal suffixes alone must never authorize a
protocol change. No v2 edit session may be attached to a run with any saved v1 edit
request/action, including a failed request. An ordinary new run with planning v1
and no legacy edit is eligible. Pending legacy requests block, never upgrade.

Retain `WorkspaceFileEdit`, `Decision::AdmittedPatch` and
`ToolCall::WriteWorkspaceFiles` unchanged for historical decoding and review.
New edits use distinct `WorkspaceTextHunk`, `WorkspaceFilePatch`,
`Decision::AdmittedTextPatch` and `ToolCall::PatchWorkspaceFiles` variants;
read/find decisions also have distinct admitted-session variants. Legacy actions
remain inspectable but are not dispatched by the v2 executor. Additive journal
schema changes must not rewrite old JSON or synthesize success. Update review,
offer and finalization variant matching explicitly rather than aliasing old types.

Exactly one of these tools is allowed per response; all argument objects reject
unknown/duplicate fields, missing fields, wrong types and coercions:

| Tool | Exact arguments | Meaning |
| --- | --- | --- |
| `read_task_text` | `path`, `start_line`, `line_count` | Positive integer, one-based line selection in a permitted file. |
| `find_task_text` | `path`, `literal` | Nonempty exact UTF-8 literal search in a permitted file. |
| `record_task_patch` | `files: [{path, expected_file_hash, hunks: [{old_utf8, new_utf8}]}]` | End the context phase with one patch set. |

Hashes are 64 lowercase hexadecimal BLAKE3 characters over complete file bytes.
Text is decoded through ordinary JSON parsing only: the HTTP JSON's arguments
string is parsed once as an arguments object; no subsequent unescaping, newline
insertion, Unicode normalization or output repair occurs. A literal backslash-n
stays two characters. CRLF, LF, BOMs and final-newline absence remain exact bytes.
Reject invalid UTF-8 and invalid JSON Unicode escapes. Empty new text is deletion
of a span, including the entire content; it does not delete the file.

### Bounds revision 1

All byte limits count UTF-8 bytes, not characters; serialized limits measure the
actual compact JSON bytes including escaping. Bounds are inclusive and cumulative
counters use checked arithmetic. Every listed limit applies independently: a
decoded-size allowance does not guarantee that arbitrary text fits its wire,
journal, context or token budget. Over-limit requests/responses fail closed; never
truncate a patch or silently increase a limit to complete the task.

| Surface | Limit | Basis and enforcement |
| --- | --- | --- |
| Preimage and postimage | 1,048,576 bytes each per target; 4,194,304 bytes each across targets | Bounded in-memory planning of larger text files; at most 8 MiB combined images. Deliberately replaces the v1 16 KiB whole-file executor limit without increasing model payload limits. |
| Complete declared snapshot | Existing 16,777,216 source bytes, 256 files, 1,024 inventory entries | `verification.rs` stays authoritative; projected postimages plus unchanged inputs must also fit before action start. |
| Files per patch | 1–8 distinct targets | Below the existing 32-path permission limit; a patch can use a subset of the grant. |
| Hunks | 1–16 per file, at most 64 total | Bounds matching, range validation and durable metadata independently of text size. |
| Old plus new text | 4,096 bytes total per patch | Combined decoded bytes, counting each occurrence in the proposal; exact anchor plus change, not complete file output. |
| Paths | 1,024 bytes each; 2,048 total in a patch | Retains the existing per-path limit with a tighter aggregate to bound JSON expansion. |
| Tool arguments / decoded reply | Serialized arguments object at most 24,000 bytes; `ModelReply` at most 60,000 bytes | Existing POST/reply ceilings; independently check both before constructing an action. |
| Durable records | Each request intent, result, observation, action intent and action artifact at most 65,536 bytes | Existing `MAX_ARTIFACT_BYTES`; action intent additionally at most 60,000 bytes to leave metadata headroom. Measure final records, not estimates. |
| Reads | At most 4 successful read/find observations per session; at most 2,048 excerpt bytes each and 6,144 total | Small exact additions to the existing 1,024-byte per-file / 4,096-byte total initial previews; observation JSON at most 16,384 bytes each. |
| Read line request | 1–128 lines, within the current file | Byte cap still applies even for one very long line. |
| Find request/result | Literal at most 256 bytes; at most 8 match locations, 256 excerpt bytes per location | Literal search only, ascending byte offsets; no regex or shell. |
| Edit model turns | At most 5 prepared attempts, including failed/never-started attempts; fifth turn patch-only | Reserves one final patch after at most four reads; counters never reset on restart. Also consumes the existing run cap of 64 attempts. |
| POST / context | Existing 24,000-byte POST; conservative `body_bytes + max_tokens + 4096 <= n_ctx` | Assemble and check the actual next request before reserving a turn. Do not drop saved observations to fit. |
| Output / deadline | Existing profile: 1–2,048 output tokens (default 512); 1–120,000 ms | Same immutable profile as planning. No output-budget increase, automatic retry or deadline reset. Existing active-time/stall gates also apply. |
| Transport capture | At most 3 artifacts of 65,536 bytes per attempt, including both metadata GETs | Existing journal and transport caps; cap chunked bodies even without Content-Length. |

The bounds intentionally admit fewer edits than every theoretical combination of
file/hunk maxima. For example, 4,096 bytes of control characters can exceed the
24,000-byte arguments limit after escaping. Chunk 3 must test escaped text and
both sides of every serialization cap, plus an emCP-shaped compact paragraph
reply. A maximum-token response is not guaranteed to be a complete valid patch;
length completion fails without writes. With five attempts, raw transport capture
is at most 983,040 bytes per session, within the unchanged per-artifact policy.

### Exact preparation and filesystem boundary

The pure planner receives admitted preimages and validated path/permission data;
it performs no filesystem or journal access. For each target it verifies the
whole-file hash, valid UTF-8 and all bounds. `old_utf8` must be nonempty and occur
exactly once in the immutable preimage, counting overlapping occurrences too
(`aa` in `aaa` is ambiguous). Resolve all ranges against that same preimage as
half-open UTF-8 byte ranges. Reject intersecting ranges, duplicate hunks,
`old_utf8 == new_utf8`, and any file whose final bytes equal its preimage. Adjacent
ranges are allowed. Never search text created by another hunk, guess offsets or
use fuzzy matching. Apply resolved hunks from highest byte offset downward.

Canonical patch order is ascending normalized path UTF-8 bytes, then ascending
resolved start offset. Preserve model order in the saved reply, but hash the
canonical prepared patch so reordering otherwise identical files/hunks gives the
same patch identity. Define the hash input as compact JSON of the ordered tuple
`[2,1,session_id,admission_id,context_id,proposal_request_id,model_request_id,permission_id,snapshot_id,
files]`, where each file is
`[path,pre_hash,post_hash,pre_size_bytes,post_size_bytes,hunks]`
and each hunk is `[start,end,old_utf8,new_utf8]`; integers are unsigned decimal,
`pre_size_bytes` and `post_size_bytes` are byte counts, not encoded file images,
and strings use `serde_json` compact escaping. Hash UTF-8 domain prefix
`shuttle-workspace-text-patch-v2\0` (ending in one NUL byte) followed by that JSON.
Identity binds this session's permission; it is not a content-only global ID.

The prepared action contains that complete canonical payload and identity, full
protocol/bounds identity, session and originating `model_request_id`. The
separate `proposal_request_id` continues to identify the planning response bound
to the permission. Persist exact
hunks, resolved ranges and pre/post hashes/sizes, not large final file images.
Postimages are constructed in bounded memory before the first write; a prepared
restart reconstructs them from freshly validated preimages and the saved payload
and requires identical ranges, hashes and identity. The completion artifact binds
the action, patch identity, paths, observed per-file posthashes and full post
snapshot. The immutable intent supplies review text without rereading source.

The workspace layer must check all of the following **for every file before the
action is marked started**, not inside a loop that already writes earlier files:

1. The run is eligible, admission/context/proposal/permission still match, the
   permission is unrevoked, and the full declared snapshot is current.
2. Paths use canonical `/`-separated normal components, match exact declared and
   granted paths, and resolve under the canonical workspace root. Reject absolute,
   UNC/drive-prefixed paths, traversal, empty/dot components, backslashes, NUL,
   Windows alternate streams (`:`), reserved device components and trailing
   dot/space components. No case folding or Unicode normalization to gain a grant.
3. Root, parent components and targets pass link/reparse checks. All targets are
   existing regular files; reject aliases resolving to the same native file ID and
   targets with link count greater than one. Failure to establish required file
   identity/type checks is an error, not permission to proceed.
4. Open each target for read/write without truncation; validate handle identity,
   size and bytes against its preimage. Retain handles through application. All
   postimages, projected snapshot sizes and durable record sizes are validated.
5. Recheck permission, full snapshot and all target path/handle bindings at the
   final pre-start boundary. Only then commit the started marker. Immediately
   before each write, repeat that target's path/type/identity/preimage checks;
   do not compare the full original snapshot after earlier files have changed.

Write in canonical path order through retained handles: seek, write exact
postimage, set length, sync. Read back every target, verify expected posthashes,
and capture the final snapshot; unchanged declared files and snapshot metadata
must still match the expected pre/post relationship. Persist success only after
all checks and syncs. No temporary-file replacement is required in v2: preserving
existing file identity/metadata avoids introducing rename/ACL semantics. Partial
write, sync, post-read or completion-commit failure after start is unknown, even
if some bytes look correct. No automatic rollback or replay.

This is transactional **journal preparation**, not a multi-file filesystem
transaction. A hostile concurrent writer can race checks, modify open files or
replace path components after inspection; transient change-and-restore and power
loss beyond OS sync guarantees are not covered. Checks detect drift at available
boundaries; they do not provide an atomic filesystem snapshot, exclusive workspace
ownership or general hostile-filesystem isolation.

### Durable bounded context session

Add a dedicated versioned admitted-edit session/observation ledger beside the
existing model-request ledger; do not fabricate fixture actions or mix reads into
transient history. The single journal owner controls session transitions. Persist
the identities above, turn/read/byte counters, each request link, terminal reason,
and at most one prepared action link. Read/find uses only currently granted,
declared regular UTF-8 files and the same path, size and freshness checks as patch
preparation. Full snapshot freshness is checked before and after access.

Observations store the exact excerpt text and its BLAKE3 hash, path, whole-file
hash, byte range(s), one-based line range(s), requested range/literal, truncation
flags, and newline metadata (`lf`, `crlf`, `mixed`, or `none`, with lone CR count
and final-LF flag). LF delimits lines; CRLF remains intact in returned text, lone
CR is content, and trailing LF does not create an extra line. An empty file has
zero lines: line reads fail as out of range, literal find returns no matches, and
anchor-based patching cannot insert into it in this revision.

Read returns the requested line span clipped to EOF and then the remaining byte
allowance at a UTF-8 scalar boundary, never between CR and LF. Record actual byte
offsets and whether the last returned line is partial; do not append a truncation
marker to exact text. An invalid start or zero remaining allowance fails. Find
scans the bounded whole preimage, including overlapping matches, and returns the
first eight start/end offsets; scan for a ninth to set `matches_truncated`. Each
excerpt starts at its match and takes at most 256 bytes (also bounded by remaining
allowance), trimmed to scalar/CRLF boundaries, with clipping recorded. Include
match-end metadata even if the excerpt is shorter than the literal; never infer
uniqueness from a truncated result. No matches is a successful empty observation.
Charge returned text bytes including duplicates in overlapping excerpts.

Every subsequent request reconstructs the exact ordered tool-call/result pairs
from durable observations, with deterministic IDs derived from session and turn.
Record a deterministic bounded initial context projection and its hash in the
session: retain objective, constraints and identity/check metadata; include file
previews only for permitted paths, using the existing preview caps. Refuse to open
a session if required context cannot fit; do not silently shorten objective or
constraints. If adding history exceeds the next POST/context/journal bound, pause
without another POST; do not discard observations or reset counters. A smaller
remaining excerpt allowance may shorten a read, with truncation recorded. The
model sees remaining budgets and the fifth turn exposes only `record_task_patch`.

Clarification (2026-09-18, Chunk 2b): "pause" above is the minimum. Any failed
turn preparation — a stale admission or permission, a changed declared input, an
unresolved workspace action, a pending request belonging to another workflow, an
exhausted turn budget, or a history/POST bound overflow — both pauses the run and
closes the session, rather than pausing a session that stays open. This is
deliberately stricter than the sentence above and is the intended behavior, for
two reasons. Most of these conditions are permanent for a given session: history
and serialized-request size only grow, and a changed admission, permission or
snapshot identity can never match the session's frozen definition again, so a
session left open could only fail again on the next turn while presenting itself
as resumable. The remainder are genuine anomalies for a single-task journal, and
classifying failures into "retryable" and "terminal" at a durable boundary would
add exactly the kind of silent misclassification surface this contract exists to
avoid. Closure preserves every observation, counter, artifact and retained
failure record and forbids only further turns, so "do not discard observations or
reset counters" continues to hold. The operational consequence is explicit: an
edit session burned this way is not reopened, and continuing the objective needs
a fresh task state with a new human admission and permission grant.

Clarification (2026-09-20, adversarial review fix F3): the rule above, and the
"failed/unknown completion closes that session" sentence earlier in this
decision, both describe a turn that failed to settle cleanly. Neither covers a
caller replaying a call against a request that already *succeeded* — for
example a duplicate `finish_admitted_text_read` for the same request ID. That
is not a G3 anomaly; it is the caller re-delivering its own already-applied
result. It is rejected the same way (`read request is settled or unknown;
replay blocked`), but must not close the session: doing so would burn
remaining turn and read budget over a duplicate call that changed nothing.
Every other settled or unresolvable state named above — unknown, failed, or a
request that never reached `started` — still closes the session exactly as
this clarification already describes. A session already closed by an earlier
failure is unaffected either way: closing an already-closed session is a
no-op.

### Non-streamed transport and commit/recovery rules

Select non-streamed `application/json` for v2 (`stream: false`, no
`stream_options`), retaining the loopback-only client, no proxy/redirect/retry,
`autoload=false`, immutable sampling profile and pre/post metadata/file identity
checks. There is no terminal token display to preserve, and this removes repeated
SSE framing without raising the 64 KiB cap. Retain exact bounded HTTP response
bytes as an artifact; do not parse a truncated prefix as a reply or rebuild raw
evidence from parsed JSON. Accept `application/json` with optional UTF-8 charset
and only absent/identity Content-Encoding; reject other encodings or charset
values. Reject unexpected content type, HTTP failure,
non-JSON trailing data, mismatched model, missing response/tool identity, multiple
choices/tools, non-function tools and finish reasons other than `tool_calls`.
Require choice index zero and assistant role. Validate usage as nonnegative
integers when present; missing usage remains unknown rather than zero. Prose and
reasoning are inert bounded raw data, never instructions or extra actions.

Reserve intent and turn before exchange, mark request started before the first
GET, then perform at most GET/POST/GET. Failed paths may have fewer exchanges.
Only a complete valid reply with successful post-identity checks is eligible for
application. For read/find, commit request result/artifacts, exact observation,
applied marker and counters in one journal transaction. For a patch, prevalidate
and construct all images, then commit request result/artifacts, prepared intent,
applied marker and session closure together. This deliberately strengthens the
current v1 two-transaction `finish_model` then prepared-action application path.
No saved successful-but-unapplied v2 patch is needed. Failure results still retain
bounded artifacts and any available usage, close the session and pause the run;
there is no automatic model correction turn.

If a complete wire-valid reply fails read/patch domain validation, freshness,
permission or record-size checks before its application transaction, commit a
bounded failure result (`reply: null`, diagnostic `error`), raw artifacts and
available usage with request state `failed`, `applied = true`, and application
`discarded:<reason>`. Close/pause the session in that same transaction, with no
observation or action. Bound the diagnostic independently so an oversized result
can still be recorded as failure. A crash or rollback before that failure commit
leaves the started request unknown; it never authorizes a second POST.

| Last durable boundary / interruption | Recovery rule |
| --- | --- |
| No request intent | No request/effect exists; normal session preparation is possible. |
| Request prepared, not started | Revalidate exact saved request, profile, identities, budgets and inputs; start that same reserved attempt once, without reserving another turn. |
| Request started, result transaction absent (including received but uncommitted reply/read) | Unknown provider completion; no inference retry and no reconstructed read. Pause. |
| Reply rejected and failure/discard transaction committed | Inspect retained failure/artifacts; session closed, no observation/action and no retry. |
| Read result/observation committed | Reuse saved text, never reread to replace it; verify current admission/snapshot/grant before any new request. |
| Patch reply and action prepared together | No filesystem effect yet. Revalidate/reconstruct all images and resume that action once; stale input/grant cancels before start. No new POST. |
| Action started, no committed success; crash before first write, between writes, or after sync | Unknown, even if files equal expected postimages. Inspect only; no automatic replay, rollback or manufactured success. |
| Action success and postimage artifact committed | Return saved result; require explicit re-admission, complete verification, review, human accept/reject and ordered finalization. Never write again. |

### Protocol examples

In this table `\n`/`\r\n` denote decoded newlines, and every file supplies the
BLAKE3 of its complete admitted preimage. Hashes/identity fields are omitted only
for readability; all rows remain subject to permission and size checks.

| Case | Preimage and proposed hunks (`old` → `new`) | Required result |
| --- | --- | --- |
| Replacement | `alpha\nbeta\n`; `beta` → `gamma` | `alpha\ngamma\n`; bytes outside the range preserved. |
| Deletion | `alpha\nbeta\n`; `beta\n` → empty | `alpha\n`; file still exists. |
| Insertion by anchor | `# Tests\r\nrun\r\n`; `# Tests\r\n` → `# Tests\r\nOffline only.\r\n` | CRLF preserved; insertion needs a unique nonempty anchor. |
| Multiple hunks | `a=1\nb=2\n`; `b=2` → `b=3`, `a=1` → `a=2` | Both resolve against original text; reverse model order has the same canonical identity. |
| Multiple files | `a.txt: one\n`, `b.txt: two\n`; `one` → `ONE`, `two` → `TWO` | Both prepared before either write; stale hash or ambiguous anchor in `b.txt` prevents any write to `a.txt`. |
| Overlapping matches | `aaa`; `aa` → `b` | Reject ambiguity even though a non-overlapping search might report one match. |
| Intersecting hunks | `abc`; `ab` → `x`, `bc` → `y` | Reject overlap before start. |
| Escape/Unicode | `café\n`; `é` → literal backslash followed by `n` | Exact literal `caf\\n` followed by the original LF; no second escape pass. |

### Threat and failure matrix

| Threat / failure | Required boundary and outcome | Qualification obligation |
| --- | --- | --- |
| Source text contains instructions; model requests shell/permission/acceptance | Strict three-tool allowlist; text stays data, no new authority | Wrong/extra tool, injected context and extra-field tests. |
| Absolute/traversing/ADS/device path, case alias or duplicate target | Canonical grant/declaration and native identity checks before preparation/start | Windows and Linux lexical/path tests, duplicate file ID and hard-link tests. |
| Symlink, junction, reparse parent, nonregular or escaped target | Check all components and opened identity; fail before any known write | Parent and target link/reparse fixtures on each supported platform. |
| Changed admission/grant/hash or revoked permission during inference | Reject in-flight reply before action; preserve attempt, pause | Drift/revocation at read, response, prepared and pre-start boundaries. |
| Missing/repeated/overlapping anchor, invalid UTF-8, empty old text or no-op | Pure planner rejects the complete set | Table-driven LF/CRLF, Unicode, overlap, adjacent, reorder and no-op cases. |
| Oversized/hostile JSON, history, source or postimage; arithmetic overflow | Independently enforce every cap before start/dispatch; fail closed | Boundary and escape-expansion tests, especially final serialized records; duplicate-key tests at tool-argument and nested file/hunk levels. |
| Missing context outside initial preview or truncated search | Durable bounded read/find only; patch independently proves anchor uniqueness | Beyond-1-KiB reads, long lines, overlapping find, EOF and budget exhaustion. |
| Legacy whole-file request/reply offered to v2 | Full protocol/purpose/type check; reject, preserve historical rendering | Old-journal inspection and cross-version replay rejection tests. |
| Worker changed, wrong content type, incomplete or length-limited reply | Bounded failure evidence; no prepared action, no retry | Mock pre/post identity, malformed/oversized/partial transport tests. |
| Failure committing request result or prepared action | Transaction rollback; started request stays unknown on reopen | Fault injection at result/observation/action insertion and commit. |
| Failure after action start, including later target drift or I/O error | Stop; unknown with no replay or rollback; earlier writes may remain | Faults before first effect, between writes, after sync and before completion commit. |
| Hostile concurrent writer / change-and-restore / power loss | Detect at available boundaries only; no atomicity or full isolation claim | Explicit limitation retained in status/review; race tests cannot prove isolation. |
| Successful patch presented as accepted or verified | Existing re-admission, suite, offer, human decision and finalization gates | End-to-end tests must reject stale evidence and model-made acceptance claims. |

### Chunk 0 review and exit gate

2026-09-16: the primary task and independent `gpt-5.6-terra:high` reviewer agree
that this contract is deterministic, bounded, backward-compatible and implementable
without claiming filesystem atomicity. Review corrections incorporated explicit
discard/failure persistence before preparation, accepted response encoding,
byte-exact profile serialization, unambiguous size fields in patch identity, and
duplicate-key parser coverage. The independent reviewer reported no remaining
blocking ambiguity in semantics, bounds, compatibility, durable reads or recovery.

Validation for this documentation-only chunk: the original r4 file hashes remained
unchanged; both saved projections were checked for consistent failure state and
usage; local documentation links resolved; the successful byte-level examples,
overlapping-match premise and bound arithmetic were checked. No production code,
worker settings or live task state changed. Runtime tests were not run because
this chunk implements no runtime behavior. Later chunks must implement and test
the obligations above before the capability is advertised or qualified live.
