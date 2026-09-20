# Run accounting and bounded stalls

Migration 0005 adds the harness-owned provider-attempt ledger, active-time spans,
progress history and one caller-directed stall resumption. The subsequent
[llama.cpp adapter](llama-adapter.md) adds an inference transport. Commands still use `Controller` and the existing
process executor, journal completion transaction and native outbox.

## Provider attempts

Every `Controller::drive` provider attempt has a stable run/ordinal request ID.
Before invoking the provider, persist its versioned intent: provider-declared
identity, decision/replan purpose, complete typed model context, current input
identity, exact permission grant and request timeout. Intent and consumption of
one of the run's 64 request slots commit together. Prepared attempts consume a
slot even if they never start. Mark started durably before invoking the provider.

Persist the reply, provider-reported token usage when available, observed elapsed
time and limitations before applying a proposal. Request intents and results are
immutable. Applying a tool proposal atomically records its application and prepares
the exact action; the shared executor then rechecks permission and inputs. Applying
the development review proposal atomically records the application and review phase.
Rejected proposals have durable discard reasons. Neither proposal can accept a task.

| Interruption or failure | Recovery |
| --- | --- |
| Before intent commits | No request slot consumed, no provider call |
| After intent, before started | Same prepared request can run after identity, context and grant checks |
| After started, before durable response | Unknown on reopen; no automatic replay or unrelated new execution |
| After durable response, before application | Reuse the saved response without another provider call |
| After application, before action dispatch | Resume the exact prepared action through the existing executor |
| Provider error, timeout or oversized response | Record a bounded failure, retain the request slot and pause |
| Inputs, provider identity or grant changed | Discard the saved proposal durably and pause without tool effects |

The default provider deadline is 120 seconds; a provider can declare a shorter
deadline, never a longer one. Intent/result JSON each has a 64 KiB storage bound;
accepted reply JSON has a 60,000-byte bound to leave room for its result envelope.
Errors are truncated on Unicode character boundaries. No partial request result
or prepared action survives a failed transaction. An unapplied request blocks
unrelated action preparation and acceptance offers, including unknown requests.

`ModelProvider::respond_accounted` wraps the existing provider boundary. Its default
calls `respond` once and reports unavailable token usage. Usage is optional rather
than zero: this is an attempt/time ledger, not a token spending guarantee or billing
reconciliation. Provider errors/timeouts may leave remote computation and usage
unknown. Adapters must expose each retry or auxiliary inference call as a separate
harness request; hidden calls are outside this contract. Before live inference,
qualify the adapter's exact server/model/template identity, serialized messages,
tool schemas, usage parsing and cancellation. The current default provider identity
is its Rust type name, suitable for the deterministic development driver only.

The llama.cpp adapter uses `prepare_request` and `respond_prepared` to bind the
exact wire request before the started marker. Version 2 intents add the serialized
request; results add provider observations with raw artifact hashes. Optional JSON
fields preserve legacy records without a new SQL migration. Up to three bounded
transport artifacts share the immutable result transaction. The first serialized
request fixes the provider profile for the run. Each attempt contains one inference
POST and at most two declared metadata GETs, all individually observed, without
retries. See the adapter contract for live evidence and limits.

## Active time

The default full-run active allowance is one hour, independent of the existing
one-hour process-only allowance. `Instant` measures active work; system wall-clock
changes and time between API calls do not replenish or consume the allowance.
Controller steps include context assembly, provider awaits, validation, pre/post
snapshots, command execution and native delivery. Standalone command dispatch,
verification preparation/freshness, acceptance review/response and finalization
delivery are also measured. Nested calls share one span and do not double charge.

Before active work, reserve the smaller of the remaining active allowance and the
remaining five-minute no-progress allowance. On return, settle the span against
observed elapsed time in one transaction. A dropped future, abrupt exit or failed
settlement retains its entire reservation. Reopening marks the span unknown and
charges its retained reservation to no-progress time exactly once. Unknown spans
remain historical evidence; unknown requests/actions independently block replay.
The same in-memory journal rejects new work after a dropped owner future until
reopened. Saved results remain readable after exhaustion.

Native recovery delivery and evidence/user-review work receive separate bounded
60-second maintenance spans when invoked outside a controller step. These spans
are charged to the same ledger even after exhaustion, so completion delivery and
inspection remain possible. They do not authorize further execution. Accepted
finalization remains sealed and recoverable when the budget is exhausted.

The active deadline can interrupt a command before its own process deadline.
An interrupted command without a committed result remains unknown; process-tree
cleanup continues to use the existing executor's cancellation/drop mechanisms.
The process-only reservation is not refunded without its observed result.

Clock accounting covers harness work after run initialization, not CLI startup,
service/workspace registration, terminal rendering, the user's thinking time,
external work or direct calls to standalone snapshot/provider helpers. Ledger
housekeeping and filesystem/OS calls are not hard real-time operations; blocking
work can overrun a deadline and observed overruns are charged. An unknown span is
a conservative reservation, not a reconstruction of elapsed time. Post-crash work
with retained reservations may require inspection or a new run rather than a
budget refund. No budget-extension or unknown-request resolution UI is provided.

Older journals retain their existing process-time charge as a lower bound on
historical active time. Their pre-migration model-response counter becomes explicit
`legacy_budget_only` placeholders with unavailable observations. Historical model /
context time cannot be reconstructed. The compatibility `consume_model_response`
API creates a labeled budget-only entry; production requests use the controller.

## Stalls and resumption

Store evidence fingerprints in the same transaction as each action's immutable
result, artifact, verification receipt and outbox. Novel observed evidence resets
the no-progress streak/time. Previously seen fingerprints remain known across
restart and resumption, so alternating already-seen actions cannot reset the streak.
Six completed actions without new evidence or five active minutes without evidence
pause the run. The earlier three-identical-process-actions pause still applies.

Process fingerprints omit action IDs, PIDs, elapsed time and delivery metadata.
Unchanged inputs with complete output use the existing exact command/result
fingerprint; a newly observed input identity also counts as evidence. Truncated
output alone cannot establish new evidence. The scripted fixture fingerprints its
typed call, before/after inputs and complete observation. This detects observed
repetition, not semantic usefulness: noise, novel commands or changing files may
count as evidence. The overall request/time budgets still apply. A long check with
no completed observation can reach the five-minute limit.

`Journal::resume_stall` records a caller request key and nonempty direction. Only
one resumption is permitted per run. It resets the streak/time, preserves total
budgets, receipts/staleness and evidence history, and supplies the direction to the
next provider request with purpose `replan`. Exact retries are idempotent; key
conflicts, exhausted budgets, pending/unknown work, non-stall pauses and second
resumptions are rejected. It is a trusted caller API, absent from model tool choices.
It does not dispatch a command or call a model itself.

```powershell
shuttle run-status --state-dir C:\path\to\state
shuttle resume-stall --state-dir C:\path\to\state --request-key direction-1 --reason "Investigate the changed runner configuration"
```

The status command shows durable totals, stall state, attempt identity/status,
observed elapsed time, reported usage and limitations. The resumption command
records direction only; the caller then resumes its existing controller workflow.
