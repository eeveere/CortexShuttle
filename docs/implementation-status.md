# Implementation status

2026-09-03 foundation; 2026-09-07 executor, verification, acceptance, accounting,
native terminal receipt and automated terminal qualification increments;
2026-09-08 published dependency pin and local llama.cpp adapter.

## Implemented

The repository now has a Rust library and CLI, a deterministic provider, a
bounded real-file fixture, a durable controller journal, a native adapter,
and a seven-state Ratatui interaction prototype. The CLI demo persists its
session/task/episode identities and four factual result Events in its dedicated
CortexWeave database. Its final state is a development review checkpoint.

CortexWeave now supplies `deliver_native(NativeDeliveryRequest)` for session,
task and episode creation plus Event delivery. Migration 0013 stores the original
request and receipt, scoped by workspace, operation and request key. Validation,
domain insertion, Event historical ordering and the receipt share one SQLite
write transaction. Key reuse with changed content conflicts. Original creation
receipts can be recovered after lifecycle changes; they are not current-state
reads. File-backed storage uses synchronous FULL for durable acknowledgements.

The host executor now runs explicit commands under scoped grants on Windows and
Linux, capturing bounded raw stdout/stderr, exit status, timeouts and cancellation.
Windows jobs and a Linux subreaper own descendant cleanup. The journal durably
accounts for process time and pauses on failures or repeated identical successful
commands. A CLI entry point exercises the same controller without a model.
See [the process contract](process-execution.md) for scope and limitations.

## Recovery evidence

| Interruption | Required and tested result |
| --- | --- |
| Before durable intent | No action or effect; a new attempt may be prepared |
| After intent, before started | Prepared state survives; permission and input are checked again |
| After started, before effect | Unknown on reopen; dispatch is blocked even if no effect occurred |
| After effect, before result | Changed fixture survives; unknown completion blocks replay |
| After durable result/outbox | Recorded result survives; only delivery is retried |
| After native commit, before acknowledgement | Original native receipt is recovered; history is not duplicated |

Additional checks cover receipt-write rollback, journal-result rollback,
concurrent native request delivery, conflicting keys/owners, ended sessions,
immutable receipts, one journal owner, blocked actions during delivery outage,
response budgets across restart, stale review inputs, and repeated demo runs.

Abrupt-exit tests additionally terminate a child process without running
destructors after the started marker and after the file edit, then reopen its
databases in the parent. Both outcomes remain unknown and cannot be replayed.

## Foundation qualification (2026-09-03)

On Windows with Rust 1.98.0:

- Shuttle: formatter, locked build/check and Clippy passed. Ten tests passed:
  eight recovery tests and two terminal-layout tests. The child-process helper
  is skipped as a standalone test and exercised by its parent recovery test.
- CortexWeave: formatter and Clippy passed; 223 library tests and six focused
  native integration tests passed, including its existing Experience cycle.
- The CLI demo ran twice from the same saved state. Both reports contained
  four actions, five model decisions and zero pending deliveries.
- Ratatui buffers were rendered at 80x24 and 120x36 in full/reduced color.
  Wide tool/permission and compact acceptance layouts were visually inspected.
  The native Windows pseudoterminal preview started successfully; this is not
  a substitute for the planned Windows Terminal and Linux review.

These foundation tests qualify the development fixture and transaction boundaries.
Host-executor qualification is recorded separately below. Neither executor nor
fixture is an OS sandbox or an atomic filesystem snapshot.

## Host-executor qualification (2026-09-07)

Rust 1.98.0, with CortexWeave fixed at
`fe60a2334be774d0bdac67d4243d604788571eb4`:

- Windows under the normal user account: all 30 tests passed (19 process,
  one CLI, eight recovery and two layout tests). Four helper entries are ignored
  when listed directly and are exercised by their parent tests.
- Linux in Docker `rust:1.98-bookworm`: all 32 tests passed (20 process,
  two CLI, eight recovery and two layout tests), with the same four helpers.
  The container used an init process, two CPUs, a 6 GiB memory limit, a read-only
  source mount and separate Cargo/target caches. No privileged or host-PID mode.
- Rust formatting and Clippy with warnings denied passed on the final code
  (Clippy checked on both Windows and Linux).

The suites exercise real effects, exact/empty/Unicode arguments, explicit
environment and stdin EOF, nonzero exit and failed launch, noisy/silent timeouts,
bounded binary output, prelaunch cancellation, descendant cancellation, abrupt
parent exit without destructors, dropped execution futures, replay blocking,
result/outbox rollback and time-budget retention, stale authorization/inputs,
cross-workspace grants, durable repetition pauses, and native concurrent launch
isolation. CLI tests verify saved-result reuse and nonzero failure status; Linux
also verifies Ctrl+C through a private foreground-style process group and cleanup
of descendants that start a new session.

Windows tests isolate legacy inherit-all fixture launches to match the one-active-
task application contract. A separate test verifies concurrent native launches.
Embedders must not race unrelated inherit-all launchers with the executor's brief
handle-inheritance interval. Native terminal interaction/restoration review is
still open; container Linux qualification does not substitute for that review.
Operating-system launch and filesystem calls can exceed polling deadlines;
observed overruns are accounted rather than hidden. Full details are in
[the process contract](process-execution.md).

## Verification increment (2026-09-07)

The harness now persists version 1 verification plans, source snapshots and
receipts through migration 0003. Named checks bind exact process specifications,
typed input manifests, exclusions and waivers to immutable content revisions.
Directory inventories detect added files. Snapshots identify source/test/config /
dependency bytes and permissions, executable/harness hashes, and local Git
HEAD/ref/index plus declared dirty working-file identities.

Verification shares the process executor. Preparation stores intent, binding and
baseline atomically; completion stores the post snapshot and receipt with the
immutable result/artifact/outbox and time accounting. Changed inputs block prepared
dispatch; observed changes during execution or later offers durably stale affected
receipts. Restart preserves valid and stale evidence, and unknown actions retain
their bindings and replay block. Waivers remain explicit and cannot count as passes.

Focused cross-platform tests cover all five input categories before dispatch,
new files, changed source during commands, later configuration changes and revert,
valid/stale/unknown restarts, exact revision offers and waiver recording, failed
checks, missing post inputs, rollback at output/result/outbox/receipt insertion, bounded
inventories/bytes and ordinary Linux symlinks / Windows junction rejection. Git
tests stage files in an isolated temporary repository without making commits.
Additional cases cover bounded noisy output and executor-observed changes restored
before the post snapshot; neither restoration nor restart revives stale evidence.

With Rust 1.98.0 and the unchanged qualified CortexWeave pin, the full locked test
suite passed on Windows (42 tests) and Docker Linux `rust:1.98-bookworm` (44 tests),
including 12 verification tests on each platform. Five helper entries are ignored
standalone and exercised by parent tests. Formatting and Clippy with warnings
denied passed on both platforms. Windows ran under the normal user account; Linux
used an init process, two CPUs, 6 GiB memory, read-only source and separate Cargo /
target caches. The final executable regular-file guard received another focused
verification-suite and formatting/lint run on both platforms.

See [the verification contract](verification.md) for API usage and limits.
Snapshots are sampled, not atomic or continuously watched. Coverage is declared;
transient restored changes, external inputs, dynamic libraries and complete runtime
attestation remain unqualified. Git metadata records state identities rather than
porcelain clean/staged classification. The verification API does not introduce
acceptance, finalization, model integration or an additional CLI command.

## Acceptance and finalization increment (2026-09-07)

Migration 0004 adds version 1 offers and explicit user decisions, active offer
identity, and awaiting-acceptance/finalizing/finalized phases. Offers require the
exact current plan/snapshot and latest passing receipt for every unwaived check,
with no pending or unknown work. Responses revalidate the offered state;
rejections, stale offers and later edits cannot silently become acceptance.

Acceptance seals execution and commits a factual user Event plus all ordered native
finalization intents in one transaction. The existing outbox delivers ordered
episode membership and closure, task completion and session closure. Its final
acknowledgement commits the finalized phase. Restart resumes delivery without
repeating verification. The CLI exposes verify/offer/review/respond/finalize;
respond requires an interactive, typed confirmation naming the offer.

Tests cover stale/deleted inputs between offer and response, valid/stale/unknown
restarts, exact revisions, waivers, required-check selection, explicit rejection,
response-key conflicts, active offers, decision/outbox rollback, interruptions
before and after every native step, final-ack rollback, native conflicts, terminal
timestamp preservation, later edits and the CLI's explicit-input boundary.

Qualification with Rust 1.98.0 and the unchanged CortexWeave pin passed:

- Windows under the normal user account: 55 tests, including 13 acceptance /
  finalization tests, with six subprocess helper entries ignored standalone.
- Docker Linux `rust:1.98-bookworm`: 57 tests, including the same 13 acceptance /
  finalization tests, with six helper entries. The container used an init process,
  two CPUs, 6 GiB memory, read-only source and separate Cargo/target caches.
- `cargo fmt --all -- --check`, `cargo test --locked` and
  `cargo clippy --locked --all-targets -- -D warnings` passed on both platforms.

Task/session completion uses exact typed-state reconciliation because the pinned
terminal APIs lack delivery keys; concurrent external lifecycle writers remain
unqualified. Acceptance binds a captured snapshot, not a frozen filesystem.
Interactive terminal usability still needs human qualification. See the
[acceptance contract](acceptance.md) for the complete guarantees and limits.

## Run accounting and stall increment (2026-09-07)

Migration 0005 records versioned provider request intents, started markers,
immutable replies/errors, optional reported token usage and durable proposal
application/discard reasons. Request-slot consumption commits with intent. Saved
responses resume without another provider call; unobserved started requests become
unknown and block replay, unrelated execution and acceptance offers. Exact context,
provider identity and permission are rechecked before using a saved request.

Monotonic active-time spans cover controller context/model/executor/snapshot work
and native delivery, with shared clocks for nested operations. Reserve time before
work, settle against observed return, and retain reservations after abrupt exit,
dropped futures or settlement rollback. Idle user time is excluded. Exhaustion
blocks new execution while charged maintenance spans preserve inspection and
accepted finalization. The earlier process-only ledger remains intact.

Observed progress fingerprints commit atomically with action results and receipts.
Six actions or five active minutes without new evidence pause durably; alternating
already-seen observations cannot reset the streak. One idempotent caller-directed
resumption supplies the next request's replan direction without resetting total
budgets, evidence history or receipt staleness. `run-status` and `resume-stall`
expose these records through the CLI without invoking an inference service.

The focused suite covers request crash boundaries, exact saved-context/grant /
provider binding, changes during requests, errors/timeouts/oversized/invalid
responses, unknown usage, dropped and abrupt-exit requests, idle exclusion, active
deadlines, exhausted-budget delivery, alternating stalls, one directed replan,
intent/start/result/application/time/progress/resumption rollback, legacy migration,
the last request slot, immutable records and CLI status/resumption. The acceptance
suite also verifies that exhausted limits cannot change an accepted run's phase
or prevent its ordered native finalization.

Final Windows qualification under the normal user account, Rust 1.98.0 and the
unchanged CortexWeave pin: 73 tests passed, including 18 run-accounting tests.
Seven subprocess helpers are ignored standalone and exercised by parent tests.
Formatting, locked tests and Clippy with warnings denied all passed.

Final Docker Linux qualification in `rust:1.98-bookworm`: 75 tests passed,
including the same 18 run-accounting tests and seven parent-invoked helpers.
Formatting, locked tests and warnings-denied Clippy passed on the final code.
The container used an init process, two CPUs, 6 GiB memory, read-only source and
the existing separate Cargo/target caches. No model service was invoked.

See [run accounting](run-accounting.md) for the policies and remaining limits.
Provider identity/usage still need real-adapter qualification. Historical general
runtime cannot be reconstructed; old process time is a lower bound and old request
counts are labeled placeholders. Progress is an observation heuristic, deadlines
are cooperative, and interrupted spans may conservatively consume unused time.
No budget-extension or unknown-request resolution interface is introduced.

## Native terminal receipts and UI qualification increment (2026-09-07)

Shuttle now delivers task completion and session closure through CortexWeave's
native keyed operations. Each effect and immutable receipt commits in one SQLite
transaction after exact state/provenance validation. Same-key retries preserve the
original receipt and timestamps. Different keys cannot claim an already-terminal
effect; patched legacy terminal writes cannot overwrite it. Session closure binds
the exact task completion receipt and nonempty run ownership metadata. Oversized
receipt artifacts and receipt-write rollback leave no partial terminal effects.
The existing Shuttle journal, ordered outbox and acknowledgement recovery remain
the sole harness delivery path.

The patch adds CortexWeave migration 0017; existing migration history is preserved.
Tests cover independent-writer races, restart and lost acknowledgements, exact
keys/content/ownership, legacy terminal writers, bounds, rollback, immutability
and workspace removal. Old unkeyed effects without native receipts remain pending
for inspection. These guarantees do not provide authorization against arbitrary
database writers or a session-wide task-admission lease. Episode version conflicts
and automatic conflict repair retain their earlier limits.

The preview now uses one scoped terminal owner and bounded input model. Real
ConPTY/PTY children exercise Unicode input, backspace, state navigation, redraw
after resize, Esc/Ctrl+C, and restoration after normal return, errors, panic
unwinding and prior raw/custom input modes. Linux also exercises bracketed paste.
Windows uses ordinary host paste; newline/tab behavior and Shift+Enter depend on
the host. The 8 KiB input limit, Unicode scalar editing, forced-termination and
terminal ownership assumptions are recorded in [terminal qualification](terminal-qualification.md).
The user's subsequent review round is recorded below.

Windows qualification under the normal user account with Rust 1.98.0 passed:
77 Shuttle tests (eight parent-invoked helper entries ignored standalone), and
293 CortexWeave tests (one existing user-selected repository evaluation ignored). Formatting,
locked tests and warnings-denied Clippy passed in both checkouts. Nine of the
CortexWeave tests specifically cover the new terminal receipt contract.

Docker Linux qualification completed on 2026-09-08 in `rust:1.98-bookworm`:
80 Shuttle tests and 294 CortexWeave tests passed, with the same eight helper
entries and one user-selected repository evaluation ignored. Formatting, locked
tests and warnings-denied Clippy passed for both repositories. The container used
an init process, two CPUs, 6 GiB memory, both source checkouts mounted read-only
and the existing separate Cargo/target caches. No model service was invoked.

This increment initially used an explicit local Cargo path patch to the sibling CortexWeave
checkout over `0b7f2946744b000ad2239d705dc6cf0f19683d94`. Its source/migrations
matched the qualified baseline before this patch. The original Git baseline stays
in the manifest, but those builds required both modified checkouts. A locked path
dependency does not freeze source bytes; distribution still requires publication
and a new qualified immutable pin. This temporary setup is superseded by the
published pin recorded below. No model-service integration was part of this increment.

## Human review follow-up: message scrolling (2026-09-08)

Human review found that new message lines disappeared below the two-row input
viewport. The viewport now follows the end of the input using Ratatui's own wrapped
line count, preserving the empty row after a newline and recalculating after resize
and backspace. The locked Ratatui 0.29 dependency enables its rendered-line-info
feature so measurement and rendering share the same wrapping rules. Manual input
navigation remains deferred.

Focused locked qualification passed seven Windows and eight Docker Linux terminal
tests, including multiline/trailing-newline, Unicode wrapping, resize and backspace
regression coverage. Formatting and warnings-denied lint passed on both platforms.
The full-suite counts above describe the preceding qualified increment.

The user approved the scrolling fix and concluded the review round with no further
issues reported, explicitly "for now at least". This is provisional UI approval;
individual platform/checklist coverage was not specified. See the
[review record](terminal-qualification.md#human-review).

## Published native dependency pin (2026-09-08)

The user committed and published the CortexWeave patch at
`754126bfc2efd6330253826c89922148d33d9915`; the remote main reference and local clean
checkout agree on that identity. Shuttle now pins this exact Git revision and
removes the sibling path override. Cargo.lock changes only CortexWeave's source
identity. The previous baseline remains historical metadata. Reproduction commands
now require only the Shuttle checkout; no Shuttle commit was made.

Windows qualification against the published pin passed all 78 Shuttle tests, with
eight subprocess helpers ignored standalone. Formatting, locked tests and
warnings-denied Clippy passed under the normal user account with Rust 1.98.0.

Docker Linux qualification against the same published pin passed all 81 Shuttle
tests, with the same eight standalone helper entries ignored. Formatting, locked
tests and warnings-denied Clippy passed in `rust:1.98-bookworm`. The container
mounted only Shuttle's source read-only, with no sibling CortexWeave mount, and
used an init process, two CPUs, 6 GiB memory and the existing Cargo/target caches.
This full run includes the input scrolling regression added during human review.

## Local llama.cpp adapter (2026-09-08)

The authorized adapter increment adds a fixed, captured local worker profile and
exact serialized request intents through the existing model ledger. Started markers
precede transport; at most one inference POST and two declared metadata GETs occur
per attempt. Proxies, redirects, retries and model autoload are disabled. Bounded
raw exchange artifacts and immutable results commit together; failed transactions
leave no partial artifacts/results and unknown completion blocks replay. Profiles
cannot change within a run. Complete, strict single-tool SSE validation precedes
the existing executor and permission checks.

Ten focused integration tests cover exact serialization, accounting, full scripted
tool flow through the adapter, interrupted intent/start/response/result recovery,
profile changes, server/file identity changes, malformed streams, missing or invalid
usage, timeout/cancellation, transactional rollback and explicit write permission.
They run against a local test server on Windows and Linux; no inference service is
required for these tests.

Full Windows qualification passed 88 tests, with eight subprocess helper entries
ignored standalone. Formatting, locked tests and warnings-denied Clippy passed
under the normal user account with Rust 1.98.0. Evidence is retained in ignored
`.shuttle/llama-final-windows.log`.

Full Docker Linux qualification passed 91 tests, with the same eight subprocess
helper entries ignored standalone. Formatting, locked tests and warnings-denied
Clippy passed in `rust:1.98-bookworm`, with only Shuttle source mounted read-only,
an init process, two CPUs, 6 GiB memory and separate Cargo/target caches. Evidence
is retained in ignored `.shuttle/llama-final-linux.log`. The published CortexWeave
revision remains pinned; no sibling override or commit was introduced.

Live Windows Qwen qualification succeeded for three independent read steps with
thinking disabled, three with thinking enabled, and one resumed check step. The
check correctly found the unchanged fixture failing. Every live call had fixture
writes disabled, and all six fixture files remained `41`. The captured worker was
`unsloth/Qwen3.5-4B-GGUF:Q4_K_M`, build `b10217-ddd4ec142`. Exact sampled identities,
usage, timings, CLI instructions and limitations are recorded in
[the adapter contract](llama-adapter.md).

Identity sampling does not attest loaded memory, runtime libraries or GPU residency.
Context admission uses a byte estimate, not exact template/tokenizer accounting.
Streaming is bounded transport capture, without live token UI. No complete live
repair, embedding qualification or user acceptance was performed in this increment.

## Factual producer and consolidation qualification (2026-09-08)

Shuttle now qualifies complete saved verification observations into native evidence
with exact action/plan/snapshot/process-artifact binding. The narrow rustc JSON
profile captures real compiler diagnostics; the unittest profile reuses the
published native helper and exact Python 3.14.7 registry entry. Independent harness
snapshots guard input stability. Stale/unknown/incomplete observations and empty or
native-ineligible test runs cannot be promoted. Qualification and delivery intent
commit together, with native import recovery through the shared outbox.

New acceptance finalizations include a consolidation step after episode closure.
The native preview is durable before automatic acceptance, and the disposition
Event is durable before delivery. Only native automatic proposals can be accepted;
review-required and typed no-result remain explicit outcomes. Existing committed
legacy finalization sequences are retained. Qualified evidence counts toward the
single-episode membership bound.

Real rustc failure/success and unittest failure/success tests cover native delivery
replay, stale/unknown actions, tests changing during capture, empty suites and
qualification rollback. Consolidation boundary tests cover automatic/review/no-result
policy and preview/result rollback. The native acceptance recovery matrix now spans
six deliveries and checks the saved no-result disposition. The Docker cancellation
test synchronizes on observed POST dispatch rather than assuming it occurs within
100 ms; provider timeout may legitimately occur before dispatch under load.

The current raw fixture history can produce native `UnsupportedPayloadContract`;
that honest no-result does not claim an automatic repair Experience. Actual user
acceptance, a complete live model repair, general Cargo test parsing, repository
producer breadth, episode rollover and embedding qualification remain ahead.
See [producer qualification](evidence-qualification.md) for contracts, commands,
vendored-helper provenance and limitations. Neither port 8080 nor 8081 was used.

Windows qualification passed 94 tests, with eight subprocess helper entries ignored
standalone. Formatting, locked tests and warnings-denied Clippy passed under the
normal user account with Rust 1.98.0 and Python 3.14.7. Evidence is retained in
ignored `.shuttle/evidence-final-windows.log`; the revised adapter timing test also
has a focused rerun log.

Docker Linux qualification passed 97 tests with the same eight standalone helper
entries ignored, plus formatting and warnings-denied Clippy. The image combined
Rust 1.98/bookworm with the pinned Python 3.14.7 image recorded in
`tests/Dockerfile.qualification`. Shuttle source was read-only, with separate
Cargo/target volumes, an init process, two CPUs and 6 GiB memory. The final run,
including the synchronized cancellation test, is retained in ignored
`.shuttle/evidence-final-linux.log`. Shuttle remains uncommitted.

## Qualified live workflow (2026-09-08)

The isolated live workflow now connects a real expected baseline assertion failure,
the shared model tool loop, real final unittest qualification and a fresh acceptance
offer. Migration 0007 preserves its configuration and stage. Exact provider and
baseline/final plans are fixed before work; separate capture outputs preserve both
observations. Definition checks precede fixture effects. Baseline continuation
retains all consumed budgets. Unknown completion blocks replay, and reopening a
completed scripted/mock run returns the same offer without another inference call.
See [live repair](live-repair.md) and decision S014.

Live qualification uses the user-managed Qwen3.5-4B Q4_K_M worker on 8080, build
`b10217-ddd4ec142`; 8081 is unused. All isolated attempts are retained under ignored
`.shuttle/qwen-repair-*/`, with no journal or budget resets:

- Attempt 01: Windows stack overflow after the first response was durably saved;
  its read action remained prepared and the file remained `41` plus LF. Nested
  workflow futures are now boxed, with an actual native CLI regression.
- Attempt 02: v1 thinking-off repeated reads. Its one recorded operator replan
  led to edits producing `42` without LF, a failed content check and repeated
  writes. It paused with 17 saved responses and no acceptance offer.
- Attempt 03: v2 compact history, thinking-off, repeated seven reads and paused;
  the file remained `41` plus LF.
- Attempt 04: v2 thinking-on completed one read, then exceeded the 64 KiB transport
  capture bound. The second request failed, no proposal was applied, and the file
  remained unchanged. The bound was retained.
- Attempt 05: v3 thinking-off used standard tool-message history. It made 16
  successful provider calls, including edits and five failing content checks,
  before the six-action stall guard paused it. Edits contained literal escape
  characters or omitted LF; the final file is `42` without LF. Provider-reported
  totals were 55,967 input and 882 output tokens. No replan was consumed.

These outcomes do not establish a successful live repair. Protocol v3 adds standard
assistant/tool message pairs reconstructed from completed actions, while retaining
the compact factual context and exact request artifacts. Changed protocol identities
cannot be substituted into prior runs. The exact-byte proposal/transport behavior
is the next investigation; these observations alone do not distinguish model
generation from the server's tool-argument parsing. No acceptance or native
finalization was performed. No live outcome was promoted into a passing receipt.

Final Windows qualification passed 99 tests with eight standalone subprocess
helpers ignored, formatting and warnings-denied Clippy, using locked dependencies.
Evidence is retained in `.shuttle/live-repair-final3-windows.log`. Earlier checks
hit artifact/linker conflicts while builds and test executables overlapped; the
final sequential build/test/lint run passed. The final source includes the bounded
definition read and v3 tool-message regression. Shuttle remains uncommitted.

Final Docker Linux qualification passed 102 tests with eight standalone subprocess
helpers ignored, formatting and warnings-denied Clippy, using locked dependencies.
The same pinned Rust/Python qualification image, read-only source mount and bounded
container resources described above were used. The final log is
`.shuttle/live-repair-final3-linux.log`. All five new live workflow/CLI tests passed
on both platforms, alongside the existing snapshot, receipt and rollback coverage.

## Exact-byte live qualification (2026-09-09)

Investigation of the five v3 replacement responses found that decoded HTTP contents
matched the immutable journal exactly: literal backslash/n or missing LF was already
present before Shuttle executed the proposal. The parser source at worker build
`ddd4ec142` does not contain the old unconditional whitespace trim; that historical
bug is not established as the cause. The source inspected is
[the pinned upstream parser](https://github.com/ggml-org/llama.cpp/blob/ddd4ec142/common/chat-peg-parser.cpp).

Protocol v5 transports replacement content as explicit UTF-8 byte integers and
retains exact executor behavior. Invalid bytes/UTF-8, legacy string arguments and
decoded replacements over 16,384 bytes fail before proposal application. Three
new decoder tests cover fragmented streams, exact whitespace/Unicode/literal
escapes and bounds. Existing transport recovery and native CLI tests now exercise
the byte schema and reconstructed byte history. See decision S015.

Attempt 06 preserves the initial v4 schema failure: HTTP 400, no proposal, no retry.
The supplied worker log showed that `maxItems: 16384` expanded beyond llama.cpp's
grammar repetition limit. V5 omits that server-side keyword and retains the strict
Shuttle decoder limit, transport bounds and a distinct provider identity.

Attempt 07 completed with the same user-managed Qwen3.5-4B Q4_K_M worker on 8080,
thinking off. Four requests produced read, replace, check and review. The model
supplied `[52,50,10]`, yielding exactly `42` plus LF. The real baseline unittest
recorded one assertion failure/exit 1; the final unittest recorded one pass/exit 0.
Both were qualified and delivered natively. The failed baseline is stale after
the edit; the final receipt and acceptance offer are fresh, with no waivers.
Provider-reported totals were 6,064 input and 262 output tokens. Reopening returned
the same offer without another request or process action. No replan was used.

Run `892f94be-5917-4506-90b8-f479d003562d` received actual user acceptance of offer
`6c136308-4f13-42a2-a669-6a8115bacb13`. Final plan revision:
`ce14f3df2183ff8a25e5790da080b1cd9c3a2735ebe7d0e52cba2a3ad999283b`.
Full review and reopened review are retained in ignored `.shuttle/qwen-repair-07.log`
and `.shuttle/qwen-repair-07-reopened.log`, with a readable review under the run
directory. Acceptance and finalization were subsequently confirmed as recorded below;
no repair Experience is claimed.

Docker initially failed before validation because of inaccessible stale Windows
socket entries. With Docker stopped, only its transient `run` directory was renamed
to `run.shuttle-backup-20260909` and recreated; the Linux engine then started.
Images, volumes and settings were not reset. The preserved runtime directory is
under the user's local Docker directory, outside the repository.

Final Windows qualification passed 102 tests with eight standalone helper entries
ignored, formatting and warnings-denied Clippy under locked dependencies. The log
is `.shuttle/live-repair-final5-windows.log`. This includes the three new byte-decoder
tests and updated adapter/CLI recovery coverage.

Final Docker Linux qualification passed 105 tests with eight standalone helper
entries ignored, formatting and warnings-denied Clippy under locked dependencies,
using the existing pinned qualification image and read-only source mount. The log
is `.shuttle/live-repair-final5-linux.log`. A final acceptance review after these
checks confirmed that trial 07's same offer remains fresh and unaccepted; it is
retained in `.shuttle/qwen-repair-07-final-review.log`. No changes were committed.

The user then accepted the exact offer through the interactive terminal. The
durable decision uses request key
`terminal/6c136308-4f13-42a2-a669-6a8115bacb13/accept` and binds the original snapshot
`9c4d31e754341eaf6dcfeae85264dce8ffb6e5e5c5bce9f8e996aec4d62f6c86`.
The journal now reports `finalized`, with user acceptance and every native
finalization step durably acknowledged. Consolidation saved the typed no-result
`unsupported_payload_contract`, identifying the material raw fixture event; its
disposition Event is `901a7302-ac95-472a-953c-88ed2ac6e6f4`. This is an explicit
native outcome, not an automatic repair Experience. The live task's verification,
actual user acceptance and finalization/disposition gate is complete.

## Milestone 1 terminal shell: durable view (2026-09-09)

The first terminal-shell slice adds `task-view`, a read-only Ratatui view of the
existing journal. It does not create a controller, executor, grant, acceptance or
finalization path. Its interactive and `--json` forms derive one shared projection
of durable run/action/accounting/delivery/acceptance data. The finalized live
repair was exercised through both forms. The command dispatcher is now boxed to
keep command-specific async futures off the Windows main-thread stack; this also
restores `run-status` and `acceptance-review`, which had exposed the same stack
overflow after the larger live-repair command set was compiled.

Live refresh, run/resume/interrupt controls, task creation and interactive review
controls remain future slices. Existing explicit CLI commands still own effects and
their recovery contracts. See terminal qualification for the view's boundaries.

Final locked qualification passed 104 Windows tests and 107 Docker Linux tests,
with eight standalone helper fixtures ignored on each platform. Formatting and
warnings-denied Clippy passed. Evidence is retained in ignored
`.shuttle/task-view-final-windows.log` and `.shuttle/task-view-final-linux.log`.
Shuttle remains uncommitted.

## Milestone 1 terminal shell: interactive stalled-task direction (2026-09-09)

`task` adds an interactive, journal-backed workspace alongside the read-only
`task-view`. It presents durable run/action/accounting/acceptance state, scrollable
history and a bounded multiline draft. A draft cannot dispatch model work or an
effect. Only a stalled run exposes the existing one-time caller-directed replan:
two explicit Ctrl+S presses record the exact bounded direction via `resume_stall`
after the terminal has restored. The operation retains all established accounting,
staleness, receipt, outbox and unknown-completion guards.

The workspace rereads its durable projection every half second. Ctrl+R exposes the
complete saved offer through the existing read-only acceptance-review path; it
does not provide a terminal acceptance or finalization control.

The workspace rereads the durable projection while open. It does not yet provide
general task creation, process or permission controls, model submission,
acceptance review/finalization controls, editor cursor navigation or accessibility
qualification. Focused Windows layout and native terminal tests cover its input,
history, explicit confirmation and terminal cleanup; full cross-platform
qualification is recorded below.

Final Windows qualification passed 106 tests with eight standalone helper fixtures
ignored. Formatting and warnings-denied Clippy passed with locked dependencies;
the output is retained in ignored `.shuttle/workspace-windows-test.log`,
`.shuttle/workspace-windows-format.log` and
`.shuttle/workspace-windows-clippy.log`. Docker Linux qualification was attempted
initially in the generic Rust image, whose Python identity cannot qualify the
native unittest producer. The final run used the pinned
`tests/Dockerfile.qualification` Rust/Python 3.14.7 image with a read-only source
mount and separate Cargo/target volumes: 109 tests passed with eight helper
fixtures ignored, formatting and warnings-denied Clippy passed. The final output
is retained in ignored `.shuttle/workspace-linux.log`; the cached local image is
`shuttle-qualification:local`. Shuttle remains uncommitted.

## Task selection, explicit workflow start, and concurrent observation (2026-09-09)

The lifecycle slice adds `task-new` for an idle isolated scripted fixture, a saved
task picker (`task`, optionally `--root`), and Ctrl+G review/confirmation to start
or resume that registered workflow. Migration 0008 binds its run to
`scripted_fixture_v1`; existing provider-backed runs cannot be adopted by this
control. The terminal and `demo` reuse the same controller path. Creation records
no provider/tool actions, and missing/existing task data is not overwritten.

This also corrects the preceding refresh implementation, which held the
controller's exclusive journal lock. `TaskReader` uses a read-only connection and
one transaction per projection; inspection can now coexist with active work and
cannot turn STARTED into UNKNOWN or consume activity budgets. Prepared, started,
unknown requests/actions and native backlog are visible. Changed or unavailable
state invalidates confirmation, repeat events cannot authorize execution, and
actual dispatch rechecks run identity and workflow under the controller lock.
Ctrl+R retains the existing revalidating review, including its possible stale
updates and active-time accounting; it is not an acceptance control.

Focused tests exercise creation/start/replay, concurrent observation and CLI reads,
unknown-work rejection, exact-run binding, confirmation invalidation, and real
ConPTY/PTY refresh/selection/confirmation/restoration. General repository task
creation, arbitrary model submission, a background runner, interrupts, persistent
drafts and editor cursor movement remain unimplemented. Initialization spans
filesystem/native/journal state and is not atomic; interrupted directories remain
for inspection. See S016 and terminal qualification for the contract.

Final locked qualification passed 112 Windows tests and 115 Docker Linux tests,
with eight standalone helper fixtures ignored on each platform. Formatting and
warnings-denied Clippy passed. Linux used the pinned Rust/Python 3.14.7 image,
read-only source and separate Cargo/target volumes. Logs are retained in ignored
`.shuttle/lifecycle-final-windows.log` and
`.shuttle/lifecycle-final-linux.log`. Shuttle remains uncommitted.

The user manually created, ran twice as an independent verification, reviewed and
approved the lifecycle flow on 2026-09-09. Both isolated scripted tasks completed
their four durable actions once and reached `awaiting_review` without pending work.

## Immutable general-task intake (2026-09-09)

`task-intake` saves a versioned intake before a general repository task can be
admitted to a controller or executor. The record contains a canonical existing
workspace, nonempty objective, declared constraints, the complete validated
verification plan, and that plan's stable revision. Migration 0009 stores the
bounded JSON record under a singleton immutable row. The task UI and JSON view
present it as `intake` with an explicit `NO WORKFLOW admitted` marker; it creates
no run, fixture, model request, process action, receipt, grant, acceptance offer
or finalization work.

The input is read through the same read-only observer as controller tasks and
survives restart. Creation rejects a missing workspace, invalid plan/text, or an
existing task directory. The record's plan revision remains the saved revision if
the caller later changes an in-memory plan or its plan file. A future admission
must recapture source inputs and explicitly bind a controller workflow; this slice
does not make the declared plan executable, watch the workspace, or provide
general repository execution. Focused tests cover immutable persistence,
revision binding, rejected creation, no fixture/run creation, and refusal by the
scripted workflow. See S017.

Final locked qualification for this increment passed 115 Windows tests and 118
Docker Linux tests, with eight standalone helper fixtures ignored on each platform.
Formatting and warnings-denied Clippy passed in both environments. Shuttle remains
uncommitted.

## General-task intake preflight (2026-09-09)

`task-preflight` reopens only an existing immutable intake under the journal owner
lock, validates its stored plan revision and canonical workspace, then captures the
declared-input `SourceSnapshot`. Migration 0010 commits the plan record, bounded
snapshot, and immutable intake/snapshot binding in one transaction. Repeated
preflights retain each distinct source state, so later source changes produce a
different saved snapshot rather than replacing the earlier observation.

Preflight remains evidence collection only: it refuses a task with a run and does
not create an action, grant, executor, process, provider request, receipt or
acceptance offer. It is not a workflow admission, permission decision, continuous
watcher, or execution authorization. Focused tests cover exact intake identity,
snapshot changes, restart-visible projection, and refusal by the scripted fixture
workflow. See S018.

Final locked qualification for this increment passed 116 Windows tests and 119
Docker Linux tests, with eight standalone helper fixtures ignored on each platform.
Formatting and warnings-denied Clippy passed in both environments. Shuttle remains
uncommitted.

## Intake-preflight staleness (2026-09-12)

Migration 0011 adds monotonic stale status to immutable intake-preflight bindings.
When `task-preflight` captures a changed declared-input state, every earlier
preflight for that intake becomes durably stale while the new snapshot is retained
as the current observation. The task view exposes the latest snapshot and count
of earlier stale observations. Staleness cannot be cleared, even if files are
later restored. No workflow, action, grant, executor, receipt or acceptance offer
is created by this refresh. See S019.

Final locked qualification for this increment passed 116 Windows tests and 119
Docker Linux tests, with eight standalone helper fixtures ignored on each platform.
Formatting and warnings-denied Clippy passed in both environments. Shuttle remains
uncommitted.

## General-workflow admission record (2026-09-12)

`task-admit` persists the fixed `general_verification_v1` contract only after
re-capturing declared inputs and proving they exactly match the latest non-stale
preflight snapshot. Migration 0012 stores the bounded immutable admission binding
to its intake, workspace, plan revision, and preflight snapshot. Changed inputs,
missing/stale preflight, duplicate admission, and post-admission preflight all
fail. The admitted task remains visibly `NO EXECUTOR admitted`; it has no run,
action, grant, process, provider request, receipt or acceptance offer.

## Controller-facing admission validation (2026-09-12)

`load_admission` is the sole controller-facing reader for the general-workflow
contract. Under journal ownership it validates the immutable admission, intake,
plan revision, preflight link and stored snapshot, then re-captures declared inputs
to reject changed state before a controller can be constructed. It returns only the
exact admission binding and still creates no run or execution effect.

## Admitted verification dispatch (2026-09-12)

`task-verify` obtains workspace and plan only through the validated admission,
rejects waived or changed inputs, and reuses the established verification intent,
dispatch, receipt and outbox path for one named check. Caller-supplied workspace
and plan paths are deliberately absent. Focused coverage verifies the exact
admission handoff, exact saved arguments, waived-check refusal, and the CLI
boundary. The shared executor's restart, stale-input, unknown-completion and
rollback coverage applies unchanged because `task-verify` introduces no parallel
command path.

## Admitted verification recovery binding (2026-09-12)

Migrations 0013 and 0014 bind a general admission to exactly one durable run,
check ID and action ID for this phase. `task-verify` persists that identity before
preparing the shared verification action. The same invocation replays its saved
action without a second process effect; different checks or action IDs conflict.
The admission reader accepts only that bound run and still rechecks declared inputs
before execution. Focused coverage exercises a real admitted subprocess twice and
confirms one effect. Final locked qualification passed 121 Windows tests and 124
Docker Linux tests, with nine standalone helper fixtures ignored on each platform.
Formatting and warnings-denied Clippy passed in both environments. Shuttle remains
uncommitted.

## Admitted multi-check verification (2026-09-12)

Migration 0015 adds an immutable per-check ledger while retaining the original
admission/run binding. `task-verify-all` runs each unwaived saved-plan check in
declaration order through the shared executor. Its action identity is stable for
the admission and check, so restarts reuse a saved result rather than dispatching
again. The runner reloads the admitted contract before every check; changed inputs
therefore block remaining checks, while existing receipts remain stale. A failed
check is recorded and the suite continues only while the admitted inputs still
match; the run resumes only from that durable failed verification action with no
pending native delivery. `task-verify-status` is read-only and reports waived, pending, passed,
failed, stale, and unknown check state with any durable receipt.

Final locked qualification for this increment passed 125 Windows tests and 128
Docker Linux tests, with nine standalone helper fixtures ignored on each platform.
Formatting and warnings-denied Clippy passed in both environments. Shuttle remains
uncommitted.

## Admitted suite-evidence gate (2026-09-12)

Migration 0016 records one immutable suite-evidence record for an admission only
when every unwaived check has a fresh passing receipt bound to the admitted plan
revision and snapshot. The record includes exact check/action/receipt identities
and the plan's explicit waivers. `task-verify-evidence` is evidence collection,
not a user-acceptance or finalization command. `task-verify-status` refreshes
receipt freshness and reports the evidence record as stale when an included receipt
or the declared inputs no longer match. Staleness is monotonic and a failed,
pending, unknown, waived-only, or changed suite cannot record readiness.

Final locked qualification for this increment passed 126 Windows tests and 129
Docker Linux tests, with nine standalone helper fixtures ignored on each platform.
Formatting and warnings-denied Clippy passed in both environments. Shuttle remains
uncommitted.

## Versioned re-admission (2026-09-12)

Migration 0017 versions admissions within the same durable run. `task-readmit`
requires the current predecessor, an idempotency key and a reason, captures the
same immutable plan against changed inputs, and atomically retires the old
baseline while saving its successor. Original results, action identities, plan
revision and waivers remain intact. Run identity and cumulative budgets do not
reset. Unknown/unfinished work, pending delivery, unresolved reservations, stalls
and acceptance state block re-admission. `task-admission-history` reads the
retained chain without recovery or dispatch.

The executor validates current admission ownership and its source baseline during
preparation and dispatch. The evidence gate rejects retired admissions and wrong
check/plan bindings. Suite replay honors existing single-check identities and
refreshes earlier receipts after later checks. Restoring earlier file contents
does not revive retired evidence; the successor must qualify independently.

Focused Windows and Docker Linux coverage verifies fresh/stale revision restart, identical-request
replay and conflicts, restoration, preserved results/accounting/waivers,
transaction rollback before and after a run exists, unknown/prepared actions,
pending delivery, stalls, and migration of the original admission and interrupted
action. Locked qualification passed 131 Windows tests and 134 Docker Linux tests,
with ten standalone subprocess fixtures ignored on each platform. Formatting and
warnings-denied Clippy passed on both platforms; final Linux lint was confirmed
on 2026-09-13 after restarting Docker. No changes were committed.

The plan/intake remains immutable; there is no general retry policy or resolution
of unknown completion. Source snapshots remain boundary observations rather than
continuous monitoring or atomic filesystem snapshots. `task-verify-status` can
persist recovery and freshness changes; earlier descriptions of it as read-only
refer only to its lack of process dispatch and are superseded by this clarification.

## Admitted-task read-only planning (2026-09-13)

Migration 0018 stores an immutable bounded planning context and model proposal for
the current admitted task. `task-plan` captures declared source previews, exact
snapshot/file identities, objective, constraints, checks and waivers, then invokes
the existing durable local-model request path with a planning-only protocol. The
sole model response is a bounded summary, proposed paths and limitations. It is
data only: no file edit, process action, permission, acceptance or finalization is
available. Saved proposals replay without another inference call.

Focused tests cover bounded declared-only context, changed-input rejection before
a request, durable replay, rollback of proposal application, restart-unknown
replay blocking, and permanent retirement/replanning after re-admission. Phase 1b
will add the separate explicit write-intent and permission boundary.

The existing user-managed Qwen 4B worker on `127.0.0.1:8080` completed one live
planning request with the fixed saved profile. The request produced a bounded
proposal for `src/acceptance.rs` and retained both context and transport evidence
in `.shuttle/admitted-planning-live-01`. It did not dispatch a process or modify a
repository file. The worker on port 8081 was not needed.

## Admitted-task explicit write permission (2026-09-13)

Migration 0019 adds a single immutable permission record for a saved phase-1a
proposal, bound to its admission, context, proposal request, snapshot and sorted,
normalized workspace-relative paths. It records the reviewing actor, timestamp
and idempotency key. `task-write-status` is a read-only projection; it displays
the proposal and status without recovering a journal, calling a worker or making a
filesystem change. `task-write-grant` requires the exact context ID and an
explicit proposed-path approval flag. `task-write-revoke` saves a separate,
irreversible revocation record and is available even after re-admission.

Permissions have no clock expiry. They are unusable when revoked, stale or when
current declared inputs do not match the admitted snapshot. Re-admission marks
old permissions stale permanently. Focused tests cover durable grant replay,
conflicting grant rejection, grant/revocation rollback, restart/read-only status,
changed inputs, unknown model requests, path traversal rejection and re-admission
staleness. No edit intent, file write, process execution, verification dispatch,
acceptance or finalization was enabled by this increment.

## Admitted-task bounded patches and verification handoff (2026-09-13)

Phase 1c adds `task-edit`, a one-shot local-model patch request scoped to the
current immutable write permission. It accepts exact bounded UTF-8 replacements
only for permitted existing declared files with matching pre-image hashes. The
saved response and prepared journal action commit together; the started marker is
durable before any write, and interrupted work recovers as unknown and cannot
replay. Writes are synced but not a multi-file filesystem transaction.

Successful changes intentionally invalidate the old admitted snapshot. Operators
must re-admit the changed source before using the existing `task-verify-all` and
`task-verify-evidence` path. Focused tests cover a permitted model patch, durable
action result, input mutation, and required re-admission. Acceptance and
finalization remain out of scope.

## Admitted-task review offer (2026-09-13)

Migration 0020 stores one immutable, request-keyed review offer for the current
admission. `task-offer` requires fresh suite evidence for the successor snapshot,
a ready settled run, and a successful admitted workspace-write action whose
post-edit snapshot exactly matches that evidence. It saves the edit action and
artifact identities, objective, full action-history hash, and exact evidence;
it does not save a user decision or dispatch native finalization.

`task-offer-review` remains available when later workspace drift would block
execution, so it can expose monotonically stale evidence rather than hiding the
previous offer. Focused coverage proves an offer is blocked before suite evidence,
then binds the exact edit after re-admission and fresh verification, replays by
request key, and becomes stale after a later declared-source change. Explicit
accept/reject, native completion, Experience claims and terminal finalization are
reserved for Phase 2b.

## Admitted-task explicit decision and finalization (2026-09-13)

Migration 0021 adds immutable task-offer decisions. `task-offer-respond` presents
the saved review in an interactive terminal and requires a typed offer-specific
accept/reject confirmation. Acceptance revalidates the current admitted baseline,
suite evidence, workspace-edit result and action history, then atomically records
the decision and queues two factual Events (the patch and user acceptance) plus
the existing ordered episode, consolidation, task and session finalization steps.
`task-finalize` only drains this saved outbox; interrupted delivery recovers
through the established receipt path without rerunning the edit or suite.

Rejection queues no finalization. Acceptance seals the admitted run and blocks
re-admission; later source drift is still visible on the retained suite evidence
but cannot mutate the saved offer or decision. Focused coverage accepts once,
replays the decision key without duplication, drains native finalization to
`finalized`, and verifies that later drift cannot re-open the task.

## End-to-end admitted-task terminal flow

The `task` workspace now presents the saved general-task intake, admission,
recovery state, actions, suite evidence, review offer, limitations and decision
with journal-derived direction. Double-confirmed shortcuts invoke the existing
durable preflight, admission, planning, permission, edit, verification, offer,
re-admission, accept/reject and finalization workflows only after restoring the
terminal. `shuttle task` can also create its initial intake from declared
workspace/objective/plan arguments, avoiding a chain of individual task commands.
Model profile and host-execution approval remain explicit launch-time inputs.

## Remaining gates

The dependency publication/pinning gate is complete. The current user review round
is complete for now; separate Windows Terminal/native Linux checklist coverage remains unrecorded.
Automated ConPTY/PTY interaction coverage does not establish that coverage.

The adapter's transport and the narrow real producer/disposition qualification are
recorded above. The isolated live repair loop is connected to actual qualified
checks, and the live repair has completed actual user acceptance and native
finalization with an explicit no-result consolidation disposition. This completes
the first live task gate for milestone 0b; it does not establish automatic repair
Experience production, general repository coding ability or the remaining manual
terminal checklist coverage.

The local Qwen and nomic services have not been reconfigured by this work.
Repository-scoped historical Experience and held-out evaluation stay in their
planned later milestones.

General multi-check verification is now available for one admitted run. Model
integration, user acceptance/finalization, worktrees, and commits remain outside
this increment.
