# Explicit acceptance and recoverable finalization

Migration 0004 adds immutable version 1 offers and user decisions, an active offer
identity, and `awaiting_acceptance`, `finalizing` and `finalized` run phases. Neither
a process exit, verification receipt, scripted model response, waiver nor delivery
retry creates acceptance implicitly. No inference/model integration, worktree or
commit operation is introduced.

## Offers and explicit responses

`Journal::create_acceptance_offer(plan_revision)` requires a ready run, no pending
native deliveries, and no prepared, started or unknown actions. Every unwaived
check needs a current passing receipt for the exact revision and the same current
snapshot. Selection uses the latest action for each check, never an older pass
after a later failure. At least one unwaived observed check is required.

The offer records the run/objective, plan revision, snapshot ID, check/action /
artifact identities, receipt hashes, observed statuses, waivers, limitations,
action-history hash, native session/task/episode bindings, episode version and
ordered action Event IDs. `acceptance_review` resolves the saved plan, snapshot and
receipts for presentation. Offers and decisions are each limited to 64 KiB;
referenced evidence retains its existing independent bounds.

Offering pauses command dispatch while the user reviews. Offers are checked on
read, restart and response. Changed or unavailable evidence durably stales the
offer and affected verification receipts; restoring files revives neither.
Invalidating the active offer releases the run for renewed verification. New offers
have new IDs. Rejecting an older stale offer cannot displace a newer active offer.
Changes to action history invalidate the offer even if source bytes are unchanged.

`record_user_response(UserResponse)` is called only by a trusted user interface
after an explicit response naming the offer ID. It records the response key,
choice, respondent label, comment, timestamp, offer hash and accepted snapshot.
The API is absent from `ToolCall` and model decisions. Respondent labels document
input; they are not authenticated identities or signatures. Embedders must enforce
their own trusted user-input boundary.

Acceptance rechecks the active offer, plan/check/receipt identities, snapshot,
action history and delivery state immediately before committing. Rejection records
the decision and returns the active run to ready without finalization. Conflicting
response keys and second decisions fail. Identical response retries return the
immutable original, including after restart or later edits; they do not accept
those later edits.

An acceptance transaction writes the decision, exact `UserAcceptance` Event intent
and all finalization intents together, sealing the run as `finalizing`. Accepted
runs reject new process actions, model responses and generic native requests.
Later edits stale verification evidence but do not rewrite the historical decision.
Finalization always refers to the captured accepted state.

## One ordered outbox

The existing `deliveries` table carries native ingress and new harness lifecycle
requests in the same total order. `flush_outbox` is shared by the controller and
finalization CLI. The sequence is:

1. Deliver the factual `UserAcceptance` Event through native ingress.
2. Associate action Events and their qualified evidence in journal order, followed by the user
   acceptance Event, with the existing episode.
3. Close the episode using its expected next version.
4. Persist the native consolidation preview, accept only automatic proposals,
   and acknowledge the durable disposition Event.
5. Complete the native task with exact run/offer/response/plan/snapshot markers.
6. End the owned session after that exact task completion is observed.

All intents exist durably before dispatch. Acknowledgements are immutable; lifecycle
receipts must match their exact requests. The last acknowledgement and `finalized`
phase commit together. Errors leave finalization pending; they do not rerun checks,
discard the decision, skip an operation or manufacture completion. Restart can
drain the remaining outbox without an executor or model.

The pinned CortexWeave revision supplies request-key recovery for Event ingress
and episode membership/closure. Retries retain exact keys, Event ordering and
expected versions. Episode membership is limited to 100 Events, so offers permit
at most 99 combined action/qualified-evidence Events plus their acceptance Event.
See [producer qualification](evidence-qualification.md) for consolidation recovery,
supported no-result outcomes and legacy finalization behavior.

The pinned CortexWeave revision adds native `CompleteTask` and `EndSession` deliveries.
Each validates its preconditions, changes the record and inserts its immutable
receipt under one immediate SQLite transaction. Task completion binds an active
task's exact prior details and workspace/session identity. Session closure requires
that exact task-completion receipt, unchanged completed task and a matching
nonempty session ownership marker. Shuttle uses the existing outbox keys and
application service; it contains no native schema or SQL.

Same-key, same-content retries return the original receipt, including timestamps,
even after later lifecycle changes. Different content or a competing terminal
writer conflicts. The patched legacy storage operations cannot overwrite terminal
tasks or already-ended sessions. Request and receipt JSON each have a 64 KiB cap;
receipt persistence failure rolls back the mutation. These are transaction and
identity guarantees, not authorization against callers who can write the database.
Concurrent admission of unrelated tasks is not serialized by a session-wide lease.

Upgrade limitation: if an older binary completed a task/session through an unkeyed
API and lost its acknowledgement, no native receipt proves ownership of that effect.
The new adapter leaves that finalization pending for inspection, even if current
details match. It does not invent a receipt or adopt another writer's completion.
Saved unexecuted outbox operations can use the new path. External episode version
changes and other conflicts also remain pending; automatic conflict repair is absent.
These operations are published and pinned at
`754126bfc2efd6330253826c89922148d33d9915`; no sibling checkout is required.

## CLI

Use the same built Shuttle executable for verification and review: its binary hash
is part of snapshot identity. Rebuilding it or switching between an embedding
executable and the CLI may stale evidence. `verify` is a thin entry point into the
existing plan/journal/process-controller APIs.

```text
shuttle verify --workspace WORKSPACE --state-dir STATE --plan plan.json --check-id unit --action-id unit-1 --approve-host-execution
shuttle acceptance-offer --state-dir STATE --plan-revision REVISION
shuttle acceptance-review --state-dir STATE --offer-id OFFER_ID
shuttle acceptance-respond --state-dir STATE --offer-id OFFER_ID --choice accept
shuttle finalize --state-dir STATE
```

`verify` prints the revision and receipt. Run each unwaived check with a distinct
action ID; the same saved ID recovers its observation rather than executing twice.
The JSON plan embeds the existing `ProcessSpec` format and declares its inputs.
Keep state and generated outputs outside declared inputs or declare exclusions.

The response command displays the full offer and requires an interactive terminal
response of exactly `accept OFFER_ID` or `reject OFFER_ID`. EOF, cancellation,
redirected input and other text do not record a decision. There is no automatic
approval flag. Acceptance immediately attempts finalization; after an outage,
`finalize` resumes the accepted run and cannot accept an unaccepted run. The CLI
uses `journal.sqlite` and `cortexweave.sqlite` in the same state directory. JSON
presentation escapes source-controlled strings and terminal control characters.

## Qualification limits

Focused Windows/Linux tests cover offer/response edits and deletion, stale restart
and revert, unknown actions, required checks/revisions, waivers, explicit rejection,
active-offer identity, ordered membership, decision/outbox rollback, every native
dispatch/acknowledgement boundary, final-ack rollback, conflicting native completion,
immutable terminal timestamps and later independent edits. CLI tests cover
verify/recover/offer/review and reject non-interactive or implicit acceptance.
Real Windows ConPTY and Docker Linux PTY tests now cover input, resizing and mode
restoration. Visual usability still needs human qualification; see
[terminal qualification](terminal-qualification.md).

Snapshots and decisions are not atomic with host filesystem writes. Sampling can
miss changes restored between observations; a writer can also change files after
the last capture but before SQLite commits the decision. Acceptance attests the
captured state, not a locked live workspace. Declared-input, Git and runtime limits
from the verification contract remain. [Run accounting](run-accounting.md) now
measures the harness review/finalization work and preserves finalization after
budget exhaustion. New finalizations record a native consolidation disposition;
this does not guarantee an Experience or introduce a continuous watcher.
