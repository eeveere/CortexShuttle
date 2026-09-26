# Architecture decisions

## S001 — Application boundary and initial repository

2026-09-03. Use `C:/den/CortexShuttle` for the separate repository, `Shuttle`
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

2026-09-16. Accepted design. Implemented against mock workers in Chunks 1-5
(2026-09-16 to 2026-09-25) and reviewed chunk by chunk, except Chunk 2a, a
mechanical build gate with no reviewer by design. Chunk 6's independent audit
found required fixes. They were applied, a separate focused re-check passed
after two small fixes, and the operator accepted Chunk 6 (all 2026-09-25).
**Qualified live and closed 2026-09-25 (Chunk 7), under an explicit operator
amendment of the exit criterion:** the protocol passed once on the real emCP
task, and the operator rejected the patch it produced. See the closure
clarification below.
The sections below are the frozen contract, and the dated clarifications record
what implementation settled. This supersedes S029's whole-file wire
representation for future admitted edits only. S015's exact-byte principle and S028–S032's human
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

Clarification (2026-09-24, Chunk 3): three wire-level details were settled
while implementing the adapter. They are refinements, not relaxations.

1. **Offered tools.** `read_task_text` and `find_task_text` are offered only
   while a read can still succeed: not on the fifth turn, and not once the read
   count or the excerpt-byte budget is exhausted. Otherwise only
   `record_task_patch` is offered. The saved request records the offered set,
   and the decoder rejects any tool not in it, including a valid v2 tool on the
   wrong turn.
2. **Envelope tolerance.** The completion body is decoded once into typed
   structures and never as a generic JSON value, so a repeated known field is an
   error rather than last-wins, including inside `usage`. Every struct-shaped
   level (envelope, choice, message, tool call, `function`, `usage`, argument
   objects, files and hunks) must be a JSON object; a positional array is
   rejected, so the wire has one representation. Unknown top-level, choice,
   message and `usage` fields (for example llama.cpp `timings` or token-detail
   breakdowns) are inert and ignored. The `function` object and every argument
   object reject unknown fields. `content` must be a
   string or null and is never interpreted. `object`, when present, must be
   `chat.completion`.
3. **Model-facing projection.** The frozen session definition stays in the
   durable context. The provider sees only the objective, constraints, allowed
   paths, and for each allowed file its hash, size and bounded preview, plus the
   remaining budgets and replayed read pairs. Identity metadata (run, admission,
   permission and profile IDs) is not sent to the model.

Clarification (2026-09-24, Chunk 4a: size-aware read commit, Chunk 3 review
R1). A session could spend reads whose replayed history made the next request
unrepresentable. Escaping multiplies text in the durable context: a backslash
costs four bytes and a control character seven. The next preparation then
failed and closed the session, with reads remaining and no patch possible.
That contradicts the fifth turn's purpose of reserving a final patch. The
following rules close the gap. Bounds revision 1 is unchanged.

1. **Measure the next turn exactly.** Before a read commits, the journal
   composes the complete next-turn request with the candidate observation
   appended. It uses the same function and provider (identity checked) that
   will prepare that turn. That turn must pass every preparation bound: the
   24,000-byte durable context, the adapter's 24,000-byte POST and `n_ctx`
   checks, and the 65,536-byte request intent.
2. **Shorten rather than overflow.** If the full allowance does not fit, the
   read commits at the largest allowance whose next turn was composed and
   passed. A deterministic search finds it. The excerpt is shortened by the
   ordinary allowance rules (scalar and CRLF boundaries) and marked truncated.
   This is the existing "a smaller remaining excerpt allowance may shorten a
   read" sentence, now bounded by the next request's size as well as by the
   remaining budget. Domain failures such as range or UTF-8 errors are still
   decided at the full allowance and still discard the turn.
3. **Decision (b): charge actual bytes.** A shortened observation consumes
   exactly the bytes it returned. The unreturned remainder stays in the
   cumulative budget.
4. **Decision (a): refuse, and keep the patch turn.** If no allowance of at
   least one byte fits, the read is refused instead of closing the session. In
   one transaction:
   - the request is committed `succeeded` and applied, with its reply and
     artifacts retained and application `refused_read:<reason>`;
   - no observation is stored and the read counters are unchanged;
   - the session's `reads_closed_reason` is set. Migration 0023 adds it; it is
     set once and never cleared.

   The session stays open and the run stays ready. Every later turn carries
   `reads_closed`: the adapter offers only `record_task_patch`, and the journal
   rejects any read on that turn, closing the session as it would for any
   invalid reply. This is a deliberate, narrow exception to the 2026-09-18
   closure rule. The refused read did not fail preparation. It is a bounded
   decline of an otherwise valid reply, and it preserves the reserved patch
   turn. The refusal commits only if the patch-only continuation itself
   composes. Otherwise the turn fails closed and the session closes.
5. **Why the continuation always fits.** `reads_closed` is always serialized
   in the turn context. Changing `false` to `true` shortens it by one byte.
   The patch-only tool set and `patch_only` flag only shrink the request, and
   turn counters stay single digits. So the turn after a refusal (same history,
   one turn later) is never larger than the refused turn, which fit. Omitting
   the field while false was tried first: a refusal after an exactly shortened
   read then overflowed by the bytes of the added field.

Together, every committed read leaves a representable next turn and every
refused read leaves a representable patch-only turn. So no sequence of
permitted reads can leave a session without a patch turn, given that turn 0
was representable, which preparation checks before any read. No saved
observation is dropped or rewritten, and no limit is raised. Adding the field
changes the canonical turn-context bytes. No v2 session exists outside tests,
and a prepared v2 request saved by an earlier build would fail the exact
recomposition check and close its session, which fails closed.

Clarification (2026-09-24, Chunks 4b–4d: preparation, application and
orchestration). These settle choices the rules above left open. They add no
capability and relax nothing, except the one attribute bit in item 4 (see
there).

1. **Identities.** The prepared action ID is
   `<run_id>/admitted-text-patch/<session_id>`. The originating request's
   application is `admitted_text_patch:<action_id>`. The session's `action_id`
   and its closure commit in the same transaction as the action. Application
   checks all three bindings before any write.
2. **Before the started marker, application's own checks cancel.** A
   drifted input, revoked grant, new hard link, failed reconstruction or any
   other failed check that application makes before the started marker
   records the action `cancelled`. The result artifact records the reason.
   Its `input_after_hash` is the literal text "unobserved", because no
   capture is claimed. The run pauses. A cancelled action is never resumed.
   The run-level guards inside the journal's `start` are the exception: the
   acceptance seal, execution budget or stall, a pending model request, or a
   run that is not ready. Failing one of them returns with the action still
   `prepared` and the run unchanged. Nothing was written, and application
   can resume once the condition clears, for example after an explicit stall
   resumption. (Wording corrected 2026-09-24, Chunk 4 review R4-2. The first
   text said every failure cancels, which the code never did.)
3. **After the started marker, failures are unknown immediately.** An I/O
   error, a target that changed before its write, a failed read-back, a post
   snapshot mismatch or a failed success commit marks the action `unknown` and
   pauses the run at once, in the same process. A crash leaves `started`, which
   restart recovery converts to `unknown`. Neither case retries or rolls back.
4. **Success means an exact snapshot match.** The post snapshot must *equal*
   the predicted one: the admitted snapshot with only the patched files' sizes
   and hashes replaced and the Git declared-working-tree identity recomputed.
   Every other file and all metadata must be unchanged. The full post snapshot
   is stored under its identity. The completion artifact binds that identity,
   the patch identity and each observed post hash.
   One narrow exception, added 2026-09-24 after Chunk 4 review R4-1. On
   Windows, a snapshot file's `permissions` is its full attribute mask, and
   NTFS sets `FILE_ATTRIBUTE_ARCHIVE` (0x20) when a file is written in place.
   A target admitted with that bit clear therefore failed the comparison
   after a byte-perfect write and ended `unknown`. That is fail-safe, but it
   burned the task state. The comparison now excludes that bit, only on the
   patched files, and recomputes the declared-working-tree identity for both
   sides from that view. It also excludes `FILE_ATTRIBUTE_NORMAL` (0x80) on
   those files. Windows reports NORMAL only when no other attribute is set,
   so it flips whenever the archive bit does and carries no information of
   its own. Excluding it, rather than predicting it set,
   also covers file systems that do not set it. Every other attribute of a
   patched file, every attribute of an unpatched file, and all other
   metadata stay exact. The stored post snapshot is the observed one, with
   the bit as the file system left it. This is the only relaxation in these
   clarifications. It matches S033's reason for writing in place: preserving
   the metadata that is the file's own, not metadata the write itself sets.
5. **One failure path for undecodable turns.** Transport or decoder errors, a
   timeout, and a decision that is neither a read nor a patch all commit
   through the same bounded failure as a domain rejection: `failed`, applied,
   `discarded:<reason>`, session closed and run paused. Transport artifacts
   are kept, as is any usage decoded before rejection, nested in
   `provider_observation`.
6. **v1 generation has stopped.** The v1 orchestrator and whole-file executor
   are removed. `task-edit` and the terminal's edit operation run v2 only.
   `WriteWorkspaceFiles` records remain readable. Offer creation and
   acceptance match `WriteWorkspaceFiles` or `PatchWorkspaceFiles` explicitly
   rather than aliasing one to the other, so an old journal already under
   review can still finish. The v1 adapter constructor is kept only so tests
   can prove that a v1 request never reaches the v2 decoder.

Clarification (2026-09-24, Chunk 5: operator review projections). This adds no
authority and changes no stored bytes; it only says what the reader may show.

1. **Read-only and saved-state-only.** The review is derived from the session
   ledger, the prepared action intent and its hash-verified completion
   artifact. It never reads the workspace, calls a model or authorizes an
   action, so a later change to a file cannot alter what a saved review says.
   The task view and `task-edit-review` read through `TaskReader`, a read-only
   connection that neither migrates nor recovers the journal, and tolerate
   journals from before migrations 0022 and 0023. The task acceptance offer
   view (and so the offer's `change` field, Ctrl+R and the offer response
   prompt) opens the journal with `Journal::open`, as that view always has, so
   it may migrate and recover the journal and refresh the offer's stale
   reason. The review content it adds is still derived from saved state only.
2. **Absent is not matched.** A file's read-back hash is reported only when the
   completion artifact records one. A missing or hash-mismatched artifact, or a
   succeeded patch whose artifact records no hash for a file, is stated as such
   and is never shown as a match.
3. **Blocked states are named.** Prepared means nothing was written. Cancelled
   repeats the saved reason and says nothing was written. Started and unknown
   say the workspace may hold any mix of old and new text and that nothing
   replays. A rejected turn shows its saved diagnostic.
4. **Bounded and escaped.** Rendered hunks show at most 1,024 bytes per side and
   160 lines, cut on a character boundary, each cut marked with the number of
   bytes or lines left out. The structured (JSON) review keeps every hunk, which
   the wire bounds already limit. Backslash, tab and carriage return are spelled
   out. Every other control (C0, DEL and C1, newline included) and every
   character in a fixed list is shown as `\u{..}`. The list is the
   bidirectional and zero-width marks and overrides, line and paragraph
   separators, soft hyphen, combining grapheme joiner, variation selectors,
   the deprecated format characters U+206A–U+206F, interlinear annotation,
   Hangul, Khmer and Mongolian fillers and separators, and the tag block
   U+E0000–U+E0FFF. It is a hand-written list, not the full Unicode `Cf`
   category, so a format character outside it is displayed as itself. A
   reason or diagnostic is cut at 240 displayed characters between escapes and
   always marked with `…`. The two JSON prints of the review (`task-edit-review
   --json` and the offer review) pass through the same escaping for everything
   `serde_json` leaves raw, so DEL, C1 controls and the list above appear as
   `\uXXXX` and the output stays valid JSON with the same value. Other JSON
   the CLI prints, such as `task-write-status`, is not passed through it.
5. **Historical actions.** A v1 whole-file action shows paths, hashes and sizes
   only, never its replacement bytes.

Clarification (2026-09-25, Chunk 6). Path rule 2 now also refuses three more
forms:
- a reserved device name whose stem has trailing spaces or dots before the
  extension (`CON .txt`);
- the superscript-digit forms of `COM` and `LPT`;
- `CONIN$` and `CONOUT$` (added by the Chunk 6 audit, A5).

All are refusals only. No grant, limit or stored byte was widened. The list is
a defensive superset. On Windows 11 build 22631, from an absolute path, which
is how targets are opened, only bare `NUL` resolves to a device. As a bare
relative name, `CON`, `COM¹`, `CONIN$` and `CONOUT$` do, and `CON .txt` does
not. Older Windows versions, and Python's `os.path.isreserved`, treat more of
these forms as reserved.

Clarification (2026-09-25, Chunk 6 audit A3). A journal can hold historical v1
edit requests that failed before any action, as the retained r4 run does.
Such a journal has no edit review, since there is no v2 session and no edit
action. So `task-edit-review` reports the recorded v1 requests instead of
saying no edit was attempted, and its `--json` `null` means only "no v2
session and no edit action". The count is a read-only query on saved
requests, through `TaskReader`.

Closure (2026-09-25, Chunk 7; operator decision). The live qualification
([record](manual-qualifications/emcp-2026-09-25-chunk7.md)) repeated r4's
objective, constraints and profile (digest `c02198cf…`, thinking off,
`max_tokens` 512) in fresh task states. Attempt 7g met Chunk 7 pass criteria
1–5:
- the patch response was 1,367 bytes, with 221 of 512 output tokens;
- one exact hunk was applied, and the observed post hash equals the
  prediction;
- permission, action, receipt, offer and decision are all inspectable;
- Shuttle started or reconfigured no endpoint and created no commit or
  worktree.

The full emCP check passed. Criterion 6 was not met: the operator rejected the
patch, which invents `npm run test:mcp` and never reaches the guidance the
objective targets. The final evidence review found no protocol defect, no
raised limit and no weakened gate. It put the wrong patch down to two causes:
- **Edit context.** Edit turns see only permitted files (the model-facing
  projection in the Chunk 3 clarification above), so `package.json` and the
  plan summary were absent. The target lines lay beyond the bytes the model
  chose to read.
- **The model's read strategy.** It read linearly and never called
  `find_task_text`.

The operator amended the exit criterion rather than reinterpreting it. The
original texts stay in the plan, marked as amended:
- **Chunk 7 criterion 6.** "The full emCP check passes and the human reviewer
  accepts the change on its merits" becomes: the full emCP check passes and the
  human reviewer makes an explicit accept or reject decision on the merits
  through the task offer. 7g meets it: the check passed, offer `721b17b8…` was
  reviewed, and the reject is recorded.
- **Intended outcome 8.** "The original emCP documentation task completes in a
  fresh task state with a compact response comfortably inside the configured
  token and transport bounds" becomes: the original task runs in a fresh task
  state to an explicit human decision, with a compact edit response comfortably
  inside those bounds. Whether the task can complete, with acceptance, is not
  claimed.

Closure does not establish:
- **Task success.** It does not show that the task can be completed, or that
  the 4B worker can complete it given enough context. That needs a separately
  scoped increment for edit-context sufficiency, followed by a comparable live
  run.
- **Live coverage of every outcome.** Outcomes 2, 5 and 6 (v1 replay refusal,
  failures before any write, unknown after interruption) and finalization were
  not exercised live. They rest on the Chunk 1–6 tests and audits.
- **Planning headroom.** The streamed v1 planning turn used 34,279 of 65,536
  transport bytes for 148 tokens in 7g, so a planning reply above about 280
  tokens would reach r4's failure mode. Planning v1 is outside this decision.
- **Chunk 7 step 9.** The Windows Terminal and native Linux manual-usability
  records were planned separately and remain open.

This closure changes no contract, bound, stored byte or code.

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

## S034 — Show admitted edit sessions read-only reference context

2026-09-26. **Accepted design; K5b and K5c implemented and independently reviewed, the
implementation awaiting the operator's acceptance.** The
operator accepted it in chat on 2026-09-26, after two independent reviews and a
revision the same day (see the review record). The design text below is frozen;
the implementation clarifications at the end record what K5b settled and one
operator amendment of Decision 3 (the path `enum` is on the patch tool only). It
is the separately scoped increment that S033's closure clarification (2026-09-25)
left open. It adds no tool, no permission and no read or write authority, and it
raises no S033 limit; it does add two new restrictive caps and restates two
inherited ones, listed in the bounds delta below. S033's protocol, hunks, bounds, permission, review and finalization
contracts, and S028–S032's human authority, stay in force, except the two S033
sentences this decision amends for the new context revision only (see
"Amendments").

### Problem

The live Chunk 7 run (attempt 7g, emCP) produced a wrong patch, and the final
evidence review put it down to the edit context and the model's read strategy. The
saved journal supports the first cause specifically:

- The plan declared two files, `AGENTS.md` (11,643 bytes) and `package.json`
  (1,395 bytes). The planning context captured a 1,024-byte preview of each. The
  `package.json` preview contains the whole `scripts` block, with no `test:mcp`.
- Opening an edit session copies the planning context but clears the preview of
  every file that is not a permitted path (`src/edit_session.rs:712-715`) before
  the definition is frozen and hashed. The model-facing projection then lists only
  allowed files (`v2_prompt`, `src/llama.rs:141`). So the edit model never saw
  `package.json`, whose scripts would have contradicted the `npm run test:mcp` it
  invented.
- The planning summary is not carried into editing at all.
- The three reads covered `AGENTS.md` bytes 0-6,087. The guidance the objective
  targets is at bytes 8,108-8,290 and 11,531-11,643. (The qualification record notes
  these offsets were taken from the journal's file, not read from emCP.)
  `find_task_text` was offered on all three read turns and never used. The system
  prompt does not say to search first.

Whether the 4B worker can do the task with enough context is not established. The
edit POST bodies of attempts 7 and 7g are identical for request 1 (the request
intents differ only in session IDs), so "twice" is one sample.

### Measured headroom (7g, read-only from the retained journal)

Sizes are UTF-8 bytes, the unit the code checks (`serde_json::to_vec` and
`body.len()`), recomputed from `model_requests` with `mode=ro&immutable=1`:

| Request | Durable context | POST body | Request intent |
| --- | --- | --- | --- |
| edit turn 0 (`request/1`) | 3,936 | 4,896 | 12,368 |
| after 1 read | 7,251 | 7,871 | 18,912 |
| after 2 reads | 10,549 | 10,828 | 25,397 |
| after 3 reads, patch turn | 13,877 | 12,971 | 30,994 |
| Limit | 24,000 | 24,000 | 65,536 |

The worker's `n_ctx` is 65,280, so the 24,000-byte context and POST bounds bind, not
`n_ctx`. A 2,048-byte read added about 3.3 KB of durable context and about 3.0 KB
of POST in 7g; the last POST step is smaller (2.1 KB) because the patch-only turn
drops two tool schemas. The total read budget is 6,144 bytes, which is exactly three
2,048-byte reads, and 7g spent it all, so its patch turn was patch-only because the
byte budget was gone. A fourth read is possible only with shorter excerpts.

With ordinary prose the 7g patch turn used 13,877 of 24,000 context bytes, leaving
about 10 KB. Ordinary text is not the binding case. Escaping is. Text is encoded at
several levels: the definition and each observation are JSON inside a string
inside the durable context, and the POST body is a string inside the request
intent. A backslash costs four bytes in the durable context and a control character
seven. So 6,144 bytes of backslashes reach about 24.6 KB of context by themselves.
A control character costs 7 bytes in the POST body too, and 9 once that body is
stored inside the request intent, about 16 per byte in the intent. For preview
control characters the 24,000-byte context and POST bounds therefore bind first, at
about 3,430 preview bytes (24,000 / 7), long before the intent reaches 4,096 x 16 =
65,536. These worst cases are why the fit rule below must be exact and tested with
hostile text.

### Decisions

1. **Keep non-permitted previews (reference files).** A context-revision-2 session
   definition retains the planning context's previews for a bounded set of
   declared files that are not permitted paths. They are already captured, already
   bounded (1,024 bytes per file and 4,096 in total, in snapshot order) and already
   part of the planning context, so no source file is opened and no new bytes are
   created. The provider sees them in a separate `reference_files` array, distinct
   from `files`.
   - **Eligible and shown.** An eligible entry is a non-permitted file whose
     preview is present and non-empty (once the 4,096-byte pool is spent, later
     files get an empty preview and are not eligible). At most 8 eligible entries
     are shown, in snapshot order. The 4,096-byte preview total is inherited.
   - **Fields.** Each entry has `path`, `kind`, `bytes`, `preview` and
     `preview_truncated`. It has no hash: a hash invites a patch on a path that
     cannot be patched, and that would be a domain rejection that closes the
     session. Reference files are read-only data: the system prompt says they
     cannot be read with the tools or changed, `read_task_text` and
     `find_task_text` stay limited to `allowed_paths`, and a patch may touch only
     permitted files.
   - **Frozen definition.** Shown entries keep their previews. Every other
     non-permitted file has its preview cleared exactly as in revision 1
     (`utf8_preview = None`, `preview_truncated = bytes != 0`), so the durable
     budget is not spent on text the model never sees. Permitted files are unchanged.
   - **Omission.** `reference_omitted` is the count of eligible entries not shown,
     because of the 8-entry cap or the fit rule. It is a count and not a list of
     paths, and revision 2 always serializes it, even when it is 0. Then dropping an
     entry always makes the definition smaller: the count keeps its width, whereas a
     count skipped when zero would let the first drop of a one-byte preview grow the
     single-encoded definition by two bytes (the same mistake S033's `reads_closed`
     clarification records and avoids). The property is stated for the frozen
     definition and for the composed durable context, POST and intent.
2. **Carry a bounded, labelled plan summary.** The definition freezes the planning
   proposal's `summary` (available at open through the saved permission view, which
   binds the proposal request), cut at a UTF-8 boundary to 1,024 bytes with a
   `plan_summary_truncated` flag. The planning decoder already caps it at 4,096
   bytes. Limitations and proposed paths are not carried. The projection labels it
   an unverified, model-written note that the task text and files override.
   - **Where the new fields live.** All of them sit at the top level of
     `AdmittedEditSessionDefinition` (`context_revision`, `plan_summary`,
     `plan_summary_truncated`, `reference_omitted`), and none inside
     `AdmittedTaskContext`, which planning shares and hashes
     (`AdmittedTaskContext::id`, `src/workspace.rs:101`).
   - **Absence means dropped.** In revision 2 an absent summary means the fit rule
     dropped it. The planning decoder guarantees a non-empty summary
     (`src/llama/stream.rs:203`), so absence is unambiguous and no separate marker
     is stored (a `dropped` marker would make the smaller candidate larger). Review
     reports the state as included, cut (the flag) or dropped (absent).
   Attempt 7's plan wrongly said `test:live` "runs the MCP stdio end-to-end test",
   so the label is part of the decision.
3. **Revision-specific system prompt.** Context revision 2 has its own system
   prompt, and revision 1 keeps its existing one unchanged. It adds that
   `find_task_text` is the cheaper way to reach text beyond the previews, that
   reference files are read-only and cannot be changed, and that new text must not
   introduce commands, scripts or paths that do not appear in text the model has
   seen. It is guidance and not a filter: Shuttle adds no semantic check of command
   names.
   - **Tool schemas constrain the path.** Revision-2 tool schemas set `path` to an
     `enum` of exactly `allowed_paths` for `read_task_text`, `find_task_text` and
     the patch's file entries; revision-1 schemas are unchanged. A read, find or
     patch path outside the permission is rejected by the journal, and that
     rejection closes the session (`src/edit_session.rs:924-928`, through
     `commit_edit_turn_failure`, about `:1029-1042`). This design shows reference
     paths, and its guidance points the model at `find_task_text`, so without the
     enum a model could search `package.json` and forfeit the permission. Whether
     llama.cpp's grammar enforces an `enum` is not established and must be tested
     (K5b). If it does not, the schema still documents the rule and the journal
     still rejects the path, so an attempted reference read or patch still closes
     the session. That is fail-closed and widens nothing. The enum repeats
     `allowed_paths` in up to three schemas, so the fit rule counts it.
   - **Turn invariance.** The revision-2 system prompt and projection are identical
     on every turn except the fields revision 1 already varies (the budget block
     and the offered tools), and nothing is added on a patch-only turn. That
     preserves S033's Chunk 4a argument (item 5) that the turn after a refused
     read is never larger than the refused turn.
4. **A separate context revision; `bounds_revision` stays 1.** The bounds table is
   unchanged, so the bounds revision does not change. The definition gains an
   optional `context_revision`, and the wire carries `"context_revision": 2` only
   for revision 2. This keeps everything that pins `bounds_revision == 1`
   untouched: the loader, observations and history parsing, the wire's checks,
   `apply`, and the global `WORKSPACE_TEXT_PATCH_BOUNDS_REVISION`, which feeds
   S033's frozen patch-identity tuple. That tuple does not change.
   - **Which revision a new session gets.** Every newly opened session is context
     revision 2. Revision 1 exists only as sessions saved by earlier code; nothing
     rewrites or upgrades them, and the provider factory is not called for them.
   - **Absent means 1.** A revision-1 definition has no new field and encodes to
     the same bytes, so its session identity and `initial_context_hash` are
     unchanged, and its saved requests, actions, review and offer are unchanged. Its
     system prompt constant is not edited.
   - **Serialization.** Every new field has `skip_serializing_if`, and a field that
     is not an `Option` also has `#[serde(default)]` for reading. This is not
     optional: the loader recomputes `session_id` from the re-serialized definition
     (`src/edit_session.rs:531-537`), so a field that re-serialized as a default
     (`"x":0`) would make every saved revision-1 session fail with "unsupported or
     changed edit session definition". Revision 2 always serializes
     `reference_omitted` (Decision 1); the skip rule is what keeps revision 1 exact.
   - **Consistency in `validate`.** `context_revision` is accepted only when absent
     or 2; an explicit 1 is a second encoding of revision 1 and is rejected. A
     revision-1 definition must carry no revision-2 field and no non-permitted
     preview. A revision-2 definition re-checks its caps on load, not only when it
     is opened: at most 8 reference entries, 1,024 bytes per preview, 4,096 preview
     bytes in total, and a 1,024-byte summary.
   - **Downgrade fails closed.** An older binary reading a revision-2 journal
     rejects the definition at load (`deny_unknown_fields`, `src/edit_session.rs:531`)
     before any code that closes a session, and its review still works because
     `load_session_review` does not read the definition (`src/edit_review.rs`, about
     `:466`). A test pins it.
   - **Reuse.** The revision is fixed per session and the session identity already
     differs between revisions. Reuse of a saved request is decided by exact intent
     equality (`src/edit_session.rs:814`) and exact serialized-request equality
     (`src/llama.rs:833-835`), not by the provider-profile rule, so a saved
     request cannot be reused across revisions.
   - **No migration.** The definition is stored as a JSON value. K5b confirms that
     no schema change is needed.
5. **No S033 limit rises; the existing fit rules decide what fits.** Every limit in
   S033's table is unchanged: the 24,000-byte context and POST, the 65,536-byte
   intent, 5 turns, 4 reads and 6,144 read bytes. New content lives inside the
   existing budget.
   - **Fit at open, with the real preparation function.** Open builds candidate
     definitions in a fixed drop order: everything; then without the summary (it is
     unverified and the files override it); then one fewer reference entry at a
     time down to none. For each candidate it computes the session identity, builds
     the provider for that identity through a factory the caller supplies, and calls
     `compose_edit_intent` for turn 0. The first candidate that composes is frozen, with the omissions recorded. The
     objective and constraints are never shortened. If no candidate composes,
     opening is refused.
   - **Only size failures move on.** A composed turn over the context, POST or
     intent bound moves to the next candidate. Every other error (a profile or
     identity mismatch, an invalid definition) is returned at once, so a wrong
     profile does not try ten candidates and report a fit failure. A refused open
     writes no session row, reserves no request and leaves the permission usable.
     Both are tested.
   - **Why a factory.** The v2 provider is built for one session identity and
     rejects any other context (`for_admitted_editing_v2` at
     `src/llama.rs:529-532`, checked at `src/llama.rs:569-572` and pinned by
     `tests/llama_edit_v2.rs`). The identity depends on which candidate is chosen, so
     open cannot take one prebuilt provider. It takes a factory called once per
     candidate, at most ten times (the full candidate, one without the summary, and
     eight entry counts down to none). The production callers build the provider
     after open from the opened session (`model_for` at `src/workspace.rs:2387`, and
     in `src/main.rs`); K5b changes that closure from `FnOnce` to `Fn` and adds the
     candidate loop. Some tests (`tests/edit_session.rs`, `tests/edit_patch.rs`,
     `tests/llama_edit_v2.rs`) call open directly with no provider and need a
     trivial factory, and `tests/llama_edit_v2.rs` moves `profile` into its closure,
     so it must clone to become `Fn`. The identity check in the wire is not
     loosened.
   - **No pure-size fallback.** A size-only estimate is rejected: a preparation
     error after the session is loaded and its provider identity checked closes the
     session (`src/edit_session.rs:821-823`; the load and identity errors at
     `:798-802` return earlier), and there is one session per permission, so a
     misjudged turn 0 would burn the task's permission and need a fresh admission
     and grant. A flat reserve also cannot work when escaping multiplies by content
     (see above).
   - **Latent revision-1 gap, fixed for revision 2 only.** Today open checks only
     the single-encoded definition (`src/edit_session.rs:742-745`), not the
     double-encoded durable context or the wire. So a revision-1 turn 0 that does
     not fit opens and is then closed at its first preparation. That contradicts
     S033's "Refuse to open a session if required context cannot fit". Revision 2
     refuses at open; revision 1 keeps its behavior and the gap is documented, not
     changed.
   - Reads still commit only when the next turn fits (S033 Chunk 4a). With a larger
     constant context a later read may be shortened where it would not have been.
     That is the intended trade and a live measurement, not a new limit.
   - A large declared-file list already consumes the durable context through file
     metadata. This decision does not change that.
6. **Review surfaces show what the model saw.** For revision 2, `task-edit-review`
   and the terminal task view report the context revision, the reference files
   (count, paths, sizes, preview bytes), `reference_omitted` and the summary state
   (included, cut, or dropped). Rendering stays escaped and size-capped as in
   Chunk 5.
   - The offer review's edit section is built from the action alone and has no
     session (`src/workspace.rs:722-729`), and the session review loader does not
     read the stored definition (`src/edit_review.rs`, `load_session_review`). K5c
     adds a read-only read of the definition on each surface's own connection: the
     offer review already holds a `Journal` (S033 Chunk 5 clarification 1), so it
     reads through that, and `task-edit-review` and the terminal task view read
     through `TaskReader`. The offer review can then carry one summary line for a
     revision-2 session.
   - New fields are skipped for revision 1, so revision-1 output is byte-identical.
     A definition that cannot be parsed is reported as unreadable and does not
     fail the review, and old journals with no session data keep working (S033
     Chunk 5).

### Amendments to S033

For context revision 2 only, this decision amends two S033 sentences. Revision 1
keeps them exactly.

- The durable-context rule "include file previews only for permitted paths" now
  also allows the bounded reference previews of Decision 1.
- Chunk 3 clarification 3 says the provider sees only the objective, constraints,
  allowed paths and, for each allowed file, its hash, size and bounded preview. For
  revision 2 it also sees `reference_files` (no hash) and the labelled summary.

### Bounds delta

No S033 limit changes. Revision 2 adds two restrictive caps (entries and summary)
and restates two inherited ones (previews), none of which loosens anything:

| Surface | Cap | Enforcement |
| --- | --- | --- |
| Reference entries | At most 8, non-empty previews only, snapshot order | Frozen at open; re-checked when the definition loads. |
| Reference preview bytes | 1,024 per file and 4,096 in total, both inherited from planning | No new bytes are captured; re-checked when the definition loads. |
| Plan summary | At most 1,024 bytes, cut on a UTF-8 boundary | Frozen at open, with a truncation flag. |

### Security and authority

Reference previews and the summary are untrusted data. Repository text may contain
instruction-like content, exactly as it already can in the planning context, and
the summary is model output. Both are labelled data in the system prompt. Neither
adds a tool, a path the model may read or write, or an authority. Output is still
one typed tool call, and a patch is still checked against the permission, the
admitted hashes and the S033 planner. A model cannot grant, widen or claim
anything through these fields. The previews were already given to the planning
model under the same admission, and their freshness is covered by the full-snapshot
checks at every session boundary.

### Alternatives considered

| Option | Verdict |
| --- | --- |
| **`bounds_revision: 2`** | Rejected for this change. The bounds are unchanged, and the revision is pinned in the loader, observations, the wire, `apply` and the global constant behind S033's patch-identity tuple. Bumping it would change that tuple and need each site listed and changed. A separate context revision touches only the definition and the wire. |
| **Read/find on non-permitted declared files** | Deferred. It would let the model reach `vitest.config.ts` or a test file on demand, but it widens read authority beyond the write permission, needs new path rules, review projections and tests, and relies on a worker that did not search when it could. Revisit if a run with this decision still fails for missing reference facts. |
| **Raise `MAX_EDIT_READ_BYTES` or excerpt caps** | Rejected. A silent limit increase, and 11,643 bytes exceeds 6,144 anyway. |
| **Grant write permission on `package.json` or tests to make them readable** | Rejected. It changes authority to fix visibility. |
| **Filter invented npm commands in patch validation** | Rejected. Domain logic in a generic harness and a new kind of gate. |
| **Rank previews by input kind in planning capture** | Out of scope. It changes planning context bytes, which S027 and the saved planning identities cover. Snapshot order is a known limit: a manifest ranked late gets no preview when earlier files use the 4,096 bytes. |
| **Size-only fit check at open** | Rejected (Decision 5). |
| **Thinking on with 1,024 tokens, or a larger local worker** | Later, single-variable experiments, only if this decision fails with the target text provably in the transcript. |

### Consequences and limits

- **What can be visible depends on the plan.** The edit model can only see files
  the plan declares. In 7g `vitest.config.ts` and the stdio end-to-end test were
  not declared, so this decision alone cannot show them. Declaring them belongs to
  the verification plan (K4) and changes its revision. It is an operator decision,
  and it also puts those files under receipt freshness.
- **A 1,024-byte preview may cut a file.** `package.json` is 1,395 bytes and its
  preview stops at 1,024. The scripts are inside it in 7g, but a manifest whose
  scripts sit later would not be.
- **Snapshot order decides who gets a preview.** With many declared files the
  4,096-byte pool goes to the first few.
- **The result stays one sample.** A live run with this decision changes the
  prompt, the context and the summary together, so it shows whether the 4B worker
  can do the task with sufficient context, not which change helped.
- **Model-written summary risk.** A wrong summary can mislead the edit turn. The
  label mitigates it and does not remove it.
- **Revision 1 keeps its latent open-time gap** (Decision 5).
- **A reference path can still close the session.** The enum in the revision-2
  tool schemas removes the invitation, but if a model still names a reference or
  other non-permitted path in a read, find or patch, the journal rejects it and
  the session closes (Decision 3). That is fail-closed and cannot widen authority,
  and it is why the enum and its test are part of the design.

### Obligations for implementation

Each is a test or review item before the increment is advertised or run live:

1. **Revision-1 golden.** A saved revision-1 session definition, serialized request,
   review and offer are byte-identical, its identity and `initial_context_hash` do
   not change, and it still opens and prepares. Its review output carries no new
   field. An older-binary downgrade test shows a revision-2 journal is rejected at
   definition load and its review still works.
2. **Revision-2 projection.** Reference files carry non-empty non-permitted
   previews and never permitted ones or a hash. The caps (8 entries, 4,096 preview
   bytes, 1,024-byte summary with the flag) hold on both sides, a multi-byte
   boundary is cut cleanly, and files beyond the cap are cleared in the frozen
   definition. `validate` rejects a revision-1 definition carrying revision-2
   fields or previews, an explicit `context_revision: 1`, and a revision-2
   definition over any cap (entries, 1,024 bytes per preview, 4,096 in total,
   summary).
3. **Fit at open.** Candidates are tried in the fixed order (full, without the
   summary, then fewer entries), each composed with the real turn-preparation
   function through the factory. Every drop step makes the frozen definition and
   the composed context, POST and intent smaller, with `reference_omitted` always
   serialized. The objective and constraints are never shortened. Opening is
   refused only when no candidate composes, and a refused open writes no row,
   reserves no request and leaves the permission usable. A non-size error (a
   wrong-profile factory) is returned at once and does not try the other
   candidates. Test with backslash-heavy,
   control-character-heavy and quote-heavy previews (the 7g `package.json`
   preview has 105 quotes), against the context, POST and intent limits.
4. **One function.** The read-commit fit check and turn preparation still use the
   same composing function, and a read shortened by the larger context is recorded
   and charged for its actual bytes. The revision-2 prompt and projection add
   nothing on a patch-only or reads-closed turn, so the turn after a refused read
   is never larger than the refused turn.
5. **No authority change.** `read_task_text`, `find_task_text` and the patch still
   reject a reference-only path, and a hostile mock that names one proves the
   journal rejects it and closes the session with no write. The revision-2 tool
   schemas carry the `allowed_paths` enum and the revision-1 schemas do not, and
   whether llama.cpp enforces the enum is tested. `apply` and the patch-identity
   tuple are byte-for-byte unchanged; the provider's session-identity check is not
   loosened. Mutation checks pin these.
6. **Review and offer.** Revision-2 sessions show the context revision, reference
   files, omissions and summary state in `task-edit-review`, the terminal view and
   the offer review. Escaping and size caps hold. Revision-1 and older journals are
   unchanged, and an unreadable definition is reported without failing.
7. **Both platforms.** Windows and Docker Linux: formatting, locked tests and
   warnings-denied Clippy, and an independent review in a fresh session before the
   operator accepts.

### Delivery

- **K5a.** This decision reviewed independently, then accepted or amended by the
  operator. Docs only.
- **K5b.** Definition context revision 2 with its serde compatibility, the
  projection, the revision-specific system prompt and the wire field, the fit-at-
  open candidate loop (with the `FnOnce` to `Fn` factory change for the callers),
  the revision-2 tool-schema `path` enum with its llama.cpp test, and the revision-1
  golden. The projection and the fit rule land together because the fit cannot be
  measured without the projection.
- **K5c.** Review and offer surfaces, including the definition reads on the offer's
  `Journal` and on `TaskReader`.
- **K5d.** Tests, both gates, independent review, operator acceptance.
- **K4 (parallel, operator).** Retire the known-broken plan file
  `.shuttle/emcp-agents-live-test-plan.json`. Decide whether the plan declares
  `vitest.config.ts` and the end-to-end test.
- **K6 (operator only).** A fresh state directory with r4's objective, constraints
  and profile: thinking off, 512 tokens, the same worker. It succeeds when the
  edit transcript contains the target spans and the `package.json` scripts, the
  patch changes those spans and references only existing scripts, the emCP check
  passes, and the operator accepts. Compare with 7g. A failure with the target text
  provably in the transcript is the input to a larger-worker experiment, not to
  this decision.

### Review record

An independent Opus : high review (fresh session, read-only, 2026-09-26) found no
path that raises a limit or widens read or write authority, and none that changes
revision-1 behavior if implemented as written. It found four factual errors in the
first draft, all corrected above: the open-time API (a single provider cannot be
built before the session identity is known; the factory replaces it), the
measurement units (the table is now UTF-8 bytes), an unreachable four-read
extrapolation (the read budget is three full reads), and a misstatement of current
open-time behavior. It also found that bumping `bounds_revision` would touch S033's
patch-identity tuple, which the separate context revision avoids.

A focused re-check of the revision (a second fresh Opus : high session, the same
day) found the first review's six main findings resolved and the measurements
table exact. It found one new High issue and nine smaller ones, all applied above:
a reference path could close the session through a read or find (the
`allowed_paths` enum, Decision 3); which revision a new session gets; where the new
fields live and how a dropped summary is recorded; turn invariance for the refused-
read argument; the review plumbing cross-reference; the serde rule; the count
always being serialized; only size failures moving to the next candidate; the
escaping arithmetic; and several overbroad statements.

Acceptance (2026-09-26, operator). The operator accepted S034 after those edits,
without a third independent review of them. Acceptance approves the design and
authorizes K5b onward; it starts no implementation, changes no code or stored
byte, and does not satisfy K5d's independent review of the implementation, K4, or
K6's live run.

### Implementation clarifications (K5b, 2026-09-26)

K5b is implemented and gated: Windows 299 passed and Docker Linux 300 passed,
with none failed and 10 ignored on each, and formatting and warnings-denied
Clippy pass on both. An independent Opus : high review (fresh session, read-only)
found no path that raises a limit, widens read or write authority, or changes a
stored revision-1 identity, and its fixes are applied. The implementation settled
these points that the text above left open or states differently:

- **`n_ctx` is a size failure.** Decision 5 names the context, POST and intent
  bounds. The adapter's conservative `body + max_tokens + 4096 <= n_ctx` check is
  typed the same way, so a worker window too small for the smallest turn 0 refuses
  the open instead of erroring on the first candidate.
- **`validate` applies the preview caps to both revisions.** The per-file (1,024)
  and total (4,096) caps are checked for revision 1 too. Preview capture and the
  clearing of non-permitted previews at open have not changed since the initial
  import, so no genuine revision-1 definition can fail them.
- **Canonical encoding at load.** The loader requires the stored definition bytes to
  equal a re-serialization of the parsed definition. The identity is recomputed
  from the re-serialized struct, so an explicit `null` or `false` field would
  otherwise load as a second encoding under the same identity. The table's update
  trigger already forbids changing a stored definition; this refuses a planted row.
- **Order at open.** Freshness is checked once, first, because every candidate
  shares the permission, the workspace root and the run. A stale binding is
  reported as one, and the provider factory is not called for it.
- **Amendment of Decision 3 (operator, 2026-09-26): the path `enum` is on the patch
  tool only.** Decision 3 as frozen above puts an `enum` of exactly `allowed_paths`
  on `read_task_text`, `find_task_text` and the patch's file entries. After the
  live probe below, the operator chose to keep it only on the patch tool's
  `files[].path`, where this worker enforced it in 4 of 4 attempts. Read and find
  carry no `enum`: it was enforced there only sometimes and was associated with
  runs that hit the token limit without a tool call. A read or find path outside
  the permission is still rejected by the journal, which closes the session, so that
  risk is reduced by the prompt and by the model's observed behaviour and not
  removed. Wherever the text above says every path is constrained, read it as the
  patch tool's paths.
- **The path enum still has a cost.** `allowed_paths` now appears in the prompt and
  in the patch schema, two copies in the POST body. A grant of many long paths (up
  to 32, each up to 1,024 bytes) can fail every candidate where revision 1 would
  have opened. That fails closed with the size message (which now says the
  permitted path list counts), and new sessions have no revision-1 fallback. It is
  untested with a long-path grant.
- **Error text.** The request-intent bound keeps the exact message revision-1
  sessions wrote into their closure reasons.

What the tests pin, and what they do not:

- **Pinned.**
  - Revision 1: a synthetic golden definition whose identity and
    `initial_context_hash` were computed before context revision 2 existed, and
    its byte-identical round trip. The revision-1 system prompt is pinned against
    the text in `HEAD` before this change, and the revision-1 prompt and the three
    tool schemas are pinned from the current code, which is unchanged for revision
    1 by construction and by two independent reads.
  - Definition rules: the `validate` matrix, the UTF-8 cut, the always-serialized
    omission count, and the fail-closed downgrade.
  - Open: drop order (the summary first, then entries, with every step shrinking
    the definition), that only size failures move on, that a refused open writes
    and reserves nothing, the freshness order, and the canonical-encoding check.
  - Fit: hostile control-character, backslash and quote previews always leave a
    first turn that composes within the context, POST and intent bounds (control
    characters force a drop), and a small `n_ctx` refuses open as a typed size
    failure.
  - Authority: a reference path in a read, find or patch is rejected and closes the
    session. The revision-2 request names its context revision, shows hashless
    reference files and the labelled note, and constrains the patch's file paths
    to the permitted set. Nothing is added on a patch-only turn.
- **Mutation checks caught** (each guard reverted, the tests failed): non-size
  errors returning at once, the drop order, the path enum, revision-1
  serialization, a hash on reference files, the context and edit-POST and `n_ctx`
  size classifications, the freshness order, and the canonical-encoding check.
- **Known gaps.**
  - The adapter's POST-limit and the request-intent size classifications are typed
    but no test reaches them at open, because hostile previews hit the
    durable-context bound first. A wrong classification there would refuse the open
    with the size message instead of trying a smaller candidate.
  - No revision-1 session is loaded into a journal and prepared through the real
    adapter. Revision-1 wire bytes are protected by the projection pins and by code
    reading, not by an end-to-end golden from a pre-change harness.
  - Whether llama.cpp enforces the path `enum` was tested live (below), and the
    answer is "not reliably". It fails closed either way, since the journal rejects
    a path outside the permission.
  - K5c was built afterwards; see its clarifications below.

### Live enum probe (2026-09-26, operator-authorized)

Standalone requests to the operator's worker (build `b10278-d52ec04a6`, Qwen3.5-4B
Q4_K_M, `n_ctx` 65,280), sent outside Shuttle's journal with the saved profile's
sampling settings (`seed` 42, temperature 0, thinking off, 512 tokens) and the real
revision-2 system prompt and tool schemas. Nothing was written to any journal or to
the repository, and the worker was not started or reconfigured. About 40 requests
in six scripts; the results below are counts of what happened, on a small sample.

- **Acceptance.** The worker accepted every schema, including a realistic 32-path
  enum (a 22,568-byte request). The grammar-expansion HTTP 400 that the `maxItems`
  keyword caused in attempt 06 did not recur.
- **Read and find: enforced unreliably.** With an explicit instruction to use a path
  outside the enum, `read_task_text` and `find_task_text` with the shipped enum kept
  the forbidden path in 5 of 10 attempts that produced a call and honoured the enum
  in 5. The identical request gave both outcomes within one sequence, and one
  attempt ran to 512 tokens with no tool call. The `enum`-without-`type`, `const`
  and `pattern` formulations each kept the forbidden path when tried once, and the
  integer bounds made no difference, so no formulation of the top-level path was
  found that is enforced reliably.
- **Patch: enforced.** The nested `record_task_patch` `files[].path` enum forced the
  path into the enum in 4 of 4 explicit attempts, including a two-file patch and a
  second forbidden path.
- **Unprompted behaviour.** In 10 tempting scenarios that did not name a forbidden
  path (reference file visible, objective mentioning it), the model never named
  one, with or without the enum.
- **A cost.** 3 of about 27 enum-bearing requests ended at 512 tokens with no tool
  call (the model reasoned in its `content` channel that the requested file is not
  allowed); none of 13 requests without the enum did. A length completion fails the
  turn and closes the session (S033). The sample is small and the two identical
  reruns of one of these were not independent.

What this means for Decision 3. The journal still rejects any path outside the
permission and closes the session, so the enum reduces the chance of that and cannot
remove it, exactly as the fail-closed clause of Decision 3 anticipated. On this
worker it is reliable only for the patch tool and may add a length-overrun risk on
read and find. The operator chose to keep it on the patch tool only, and K5b now
ships it there (see the amendment above).

### Implementation clarifications (K5c, 2026-09-26)

K5c implements Decision 6 and is gated: Windows 309 passed and Docker Linux 310 passed, with none failed and 10 ignored on each; formatting and warnings-denied
Clippy pass on both. An independent Opus : high review (fresh session, read-only)
found no path by which the review reads the workspace, calls a model or authorizes
anything, and none that changes revision-1 or older-journal output. Its findings
are applied:

- **What is shown.** `EditSessionReview` gains an optional `context`: the context
  revision, each reference file (path, kind, size, preview bytes, truncated), the
  omitted count and the plan-summary state (included, cut or dropped, with bytes).
  It renders in `task-edit-review` (text and `--json`) and in the terminal task
  view, which builds its rows from the same lines. The offer view gains an optional
  one-line `edit_context`, printed before the offer JSON. Every rendered string is
  escaped and capped as in Chunk 5, and the JSON goes through the terminal-safe
  encoder. A path keeps a newline as an escape, and a Linux filename containing a
  backslash shows with a slash, as the model was shown it.
- **Revision 1 and older journals are unchanged.** Every new field is skipped when
  absent, so their output is byte for byte what it was; a JSON-shape test pins the
  session review, and the historical whole-file offer test asserts its output has
  no context field at all.
- **The offer's line is bound to its edit.** The offer asks for the session that
  prepared the offered action (`admitted_edit_sessions.action_id`), not the most
  recent session. An offer normally binds an edit prepared by the predecessor
  admission's session, so recency could print a later session's context beside
  that edit on the accept surface. The line says "the session that prepared this
  edit". The session review and the terminal task view still report the latest
  session beside the latest change, the existing S033 pattern, which is unchanged.
- **Only genuine revision 1 shows nothing.** The definition is validated, and its
  encoding required to be canonical, before its revision is trusted, so an explicit
  revision 1, an unknown revision, a revision-1 definition carrying revision-2
  content and a second encoding of valid content are reported as unreadable, as
  the loader would refuse them. An unreadable context claims nothing: its
  revision, counts and summary state are absent from the JSON.
- **Old and odd shapes never fail a review.** A definition column that is missing,
  NULL or the wrong type gives no context or an unreadable note, and a missing
  action column gives the offer no line. An existing test caught a first version
  that assumed the column.
- **Decision 4's downgrade sentence** ("its review still works because
  `load_session_review` does not read the definition") describes older binaries,
  which never read it. The K5c review does read it, and reports an unreadable one
  instead of failing.
- **Not changed, flagged.** `task-offer` and `task-offer-review` print their offer
  JSON without the terminal-safe encoder, so existing edit hunks there can carry
  raw bidirectional characters. The K5c line is pre-escaped and adds no hazard.

Tests and mutation checks: unit tests for every field, state and hostile input;
database-level loader tests over two sessions and every old shape; reader,
task-view, offer and real-process (`task-edit-review`, text and `--json`) tests.
Reverting any of the following was caught: the two serde skips, revision 1 showing
a context, validation before the revision branch, the offer's action binding, the
old-journal column guard, the BLOB cast, path escaping, and the offer wiring.

Not covered: the terminal widget's own drawing of the extra rows (they come from
the same lines the task view already renders), and an offer over a real two-session
journal end to end (the binding is tested over stored rows).

### Status against the obligations (K5d review, 2026-09-26)

The independent review of the whole S034 implementation, against the seven
obligations above. "Partly" means the guarantee holds but a listed part is
unproven; nothing here claims more than the tests show.

| # | Obligation | Status | What is proven, and what is not |
| --- | --- | --- | --- |
| 1 | Revision-1 golden and downgrade | Partly | Golden identity and byte round trip, the pinned system prompt (against `HEAD`), prompt and schemas, the fail-closed older-reader shape, and revision-1 review and offer output. Not proven: an older binary's review actually working (argued), and a revision-1 session opened and prepared end to end. |
| 2 | Revision-2 projection and caps | Met | The validation matrix, the UTF-8 cut, the cap of eight with the rest cleared, hashless reference files and the labelled note. |
| 3 | Fit at open | Partly | Drop order, only size failures move on, a refused open writes nothing, freshness first, small `n_ctx`, and hostile control-character previews forcing a drop. Not proven: monotone shrinking of the composed context, POST and intent through the real adapter (it is asserted for the definition, with a synthetic provider), a drop forced by backslash or quote previews, and the adapter POST and intent classifications. |
| 4 | One function, turn invariance | Partly | Turn invariance is pinned, and the S033 read-fit tests now run on revision-2 sessions. No test targets a read shortened because of the reference context. |
| 5 | No authority change | Mostly | Read, find and patch on a reference path are rejected and close the session, the patch-only `enum` is pinned, and `apply` and the patch-identity tuple are unchanged by the diff. Not proven: mutation pins on `apply` and the tuple, and a hostile mock worker through the adapter (rejections are injected at the journal boundary). |
| 6 | Review and offer | Met, except the widget | The surfaces, the offer binding, escaping, the unreadable cases and a real-process test. |
| 7 | Both platforms | Met | Windows 309 passed and Docker Linux 310 passed, with none failed and 10 ignored on each; formatting and warnings-denied Clippy pass on both. |
