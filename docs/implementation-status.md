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

## Admitted-edit v2 protocol: bounded text-patch sessions (2026-09-15 to 2026-09-20)

The retained task at `.shuttle/manual-emcp-2026-09-15-r4` exercised intake,
preflight, admission, read-only planning, path permission and the beginning of
editing; its permitted patch request reached the pinned worker and returned
HTTP 200, but the raw streamed response crossed Shuttle's 65,536-byte transport
bound before a complete response and usage record arrived, and Shuttle paused
the task without applying a workspace action. This is a protocol problem, not
an emCP problem: the v1 admitted-edit protocol requires the model to return
every byte of every replacement file as JSON integers, so a small change to an
approximately 11.6 KiB file such as `AGENTS.md` exceeds the configured output
budget and transport envelope.
[`docs/plans/DO-WE-REALLY-HAVE-TO-REPLACE-THE-WHOLE-FILE.md`](plans/DO-WE-REALLY-HAVE-TO-REPLACE-THE-WHOLE-FILE.md)
records the resulting v2 bounded transactional text-patch protocol and an
eight-chunk delivery plan (Chunk 0 through Chunk 7) that replaces it while
keeping the existing admission, permission, durability, freshness, review and
verification boundaries.

Chunk 2a's build gate (2026-09-18) recorded a toolchain deviation to native
Rust 1.98 (matching `tests/Dockerfile.qualification`'s pin) and one mechanical
Clippy fix. Chunk 2b (2026-09-18) added the durable session ledger: migration
0022 creates the `admitted_edit_sessions`, `admitted_edit_turns` and
`admitted_edit_observations` tables with immutability, retention and
monotonic-counter triggers, backing a new journal-owned session lifecycle in
`src/edit_session.rs` (`AdmittedEditSession`, `fresh_edit_session`,
`open_admitted_edit_session`, `prepare_admitted_edit_turn`,
`finish_admitted_text_read`, `close_edit_session`, `read_admitted_file`).

An independent adversarial review of Chunk 2b (2026-09-20) found the
canonicalized-root freshness binding was already correct (a misdiagnosis, not
a gap), confirmed that read/turn-budget closure is intended behavior and
recorded that in S033's clarifications (`docs/decisions.md`, 2026-09-18 and
2026-09-20), and required three fixes, all applied the same day: [F1] splits
the combined read-turn-budget check into an independently testable
`ensure_read_turn_available` function so each clause is pinned by its own
test; [F2] adds a regression test proving drift introduced after a session
starts is still caught at the read boundary; [F3] separates "settled and
applied" from merely "settled" in `finish_admitted_text_read`'s early-bail
path, so a successful replay is rejected without closing the session while an
unknown settled request still leaves it closed. `tests/edit_session.rs` grew
from 13 to 15 cases; `cargo fmt --check`, `cargo check --locked`,
`cargo clippy --locked --all-targets -- -D warnings` and `cargo test --locked`
all pass. This closes Chunk 2b; Chunk 3 (the local-model v2 wire protocol) is
next.

Correction (2026-09-24): the "`cargo test --locked` all pass" above is not
supported by retained evidence. No log of the 2026-09-20 run exists. The
recorded 2b gate ran focused test binaries only. Every environment used for
Chunks 2a–3 had Python 3.11 instead of the pinned 3.14.7, in which five
pinned-Python tests (`evidence_qualification::real_unittest_callbacks_…`, three
`live_repair` tests and `live_repair_cli`) fail with
`test_profile_component_mismatch`. The first full locked suite on a tree
containing Chunk 2b ran on 2026-09-24, together with Chunk 3. It passed on
Windows MSVC (201 passed, 10 ignored helpers) and in the pinned
`tests/Dockerfile.qualification` Rust 1.98/Python 3.14.7 image (204 passed, 10
ignored helpers), with formatting and warnings-denied Clippy passing on both.
Logs are retained in ignored `.shuttle/reconcile-windows-gate.log` and
`.shuttle/reconcile-docker-linux.log`.

Chunk 3 (2026-09-24) added the S033 local-model v2 wire adapter. It covers the
`shuttle-llama-admitted-editing-v2:<profile_digest>` provider bound to one edit
session, a shared typed turn context (`AdmittedEditTurnContext`) and a strict
parser for it, and three strict tool schemas. Requests are non-streamed
`application/json`, with the POST and conservative `n_ctx` checks. Response
headers are checked (content type, charset and encoding), and a new decoder
(`src/llama/completion.rs`) parses the body once. That decoder rejects
duplicate keys at every level and enforces every S033 wire and decoded bound
through a validator it shares with the pure planner. It also offers tools per
turn (patch-only when reads cannot succeed) and cannot receive a v1 request.
`tests/llama_edit_v2.rs` drives it through the real journal session against a
mock worker. A synthetic emCP-shaped paragraph edit measured 556 argument bytes
and a 1,066-byte response. The tool-call tokens are strictly bounded only
against the 2,048 output maximum; fit under the default 512 is an estimate until
the live run. fmt, check, Clippy (warnings denied) and the focused tests pass on
native Rust 1.98.0, run in Claude's cloud workspace rather than on the operator
machine (recorded in the plan). Orchestration, durable patch preparation and
application are Chunk 4. The Chunk 3 review (Sonnet, high) is pending.

Correction (2026-09-24): the sentence above was stale when it was committed.
Commit `d0bd549` already contains the Chunk 3 review's fixes. Its message
cites the review's R1, and its test counts grew to 22 decoder and 6 mock-HTTP
tests. No separate review record is retained.

Chunk 4 (2026-09-24, split into 4a–4d) connects the v2 protocol to the
workspace. Its fresh-task adversarial review ran, its fixes are applied, and
the operator accepted it on 2026-09-24.

- **4a:** a read commits only if the exact next request still fits. The
  excerpt is shortened if necessary. If even a minimal excerpt cannot fit, the
  read is refused and reads close (migration 0023), keeping a patch turn
  available.
- **4b:** a patch reply is resolved against freshly opened admitted
  preimages. It then commits together with its prepared `PatchWorkspaceFiles`
  action and the session's closure, before any write.
- **4c:** application revalidates every binding and reconstructs the patch
  exactly. It commits the started marker only after a final permission,
  snapshot and handle check. It writes each target through its validated
  handle, reads every target back, and records success only when the whole
  declared snapshot equals the predicted postimage snapshot. On Windows, the
  archive attribute of the patched files is excluded from that comparison,
  because the write sets it (review R4-1). A failed check before start
  cancels the action. The journal's own run-level start guards instead leave
  it prepared and resumable (review R4-2). A failure after start is unknown,
  with no retry or rollback. Targets are refused by type before they are
  opened. On Linux they are opened non-blocking and without a controlling
  terminal, so a FIFO swapped in during a race cannot hang the process
  (review R4-3).
- **4d:** `task-edit` and the terminal now run the v2 loop. v1 whole-file
  requests are no longer generated. Offer and acceptance accept either edit
  variant explicitly.

The same work fixed a pre-existing task-view bug. `edit_done` matched
`WriteWorkspaceFiles` against snake-case JSON and never fired, so a
successfully edited task was told to request the patch again instead of to
re-admit.

`tests/edit_patch.rs` covers fault injection at every S033 interruption
point, and mutation checks confirm that the key guards are pinned.

- The full v2 lifecycle (patch, re-admission, suite, offer, acceptance,
  finalization) passes in `tests/task_planning.rs` against a scripted
  provider.
- The edit workflow alone (find, patch, apply, and a rejected hostile reply)
  passes in `tests/llama_edit_v2.rs` through the real adapter against the
  mock worker.

Nothing has been observed against a real llama.cpp yet (Chunk 7). The gates
are recorded in the plan.

Chunk 5 (2026-09-24) adds the operator review surfaces for v2 edits
(`src/edit_review.rs`). It is a read-only projection: no stored byte or
authority changed.

- The terminal task view, the new `task-edit-review [--json]` command and the
  task acceptance offer now show the session ledger (turns, reads, refusals,
  rejected turns) and the edit itself: file and hunk counts, pre/post hashes and
  sizes, the exact compact hunks, and the read-back result per file.
- Prepared, cancelled and unknown actions say what did and did not happen. An
  unknown action also adds an inspect-the-workspace line to the pending list.
- Saved text is escaped and every rendered size is capped with an explicit
  marker. Historical whole-file actions render as file summaries only.
- The offer confirmation prompt said "review the exact task edit above" but the
  offer showed only IDs. It now prints the edit first. Ctrl+R on a general-task
  offer called the generic offer review, which cannot find a task offer. It now
  prints the task offer review.
- The action list now reads "text patch, N file(s), M hunk(s)" or "whole-file
  write, N file(s) (historical)".

Chunk 5's Sonnet : high review ran the same day and its fixes (R5-1 escape
set and terminal-safe JSON, R5-2 reason cut marker, R5-3 doc scope and a
real older-journal test, R5-4 unrecorded read-back line, R5-5 dead arm) are
applied. The operator accepted Chunk 5 on 2026-09-25. That acceptance does not
satisfy Chunk 6's independent audit or Chunk 7's live qualification.

Chunk 6 (2026-09-25) consolidates documentation, tests and the gates. It
adds no capability and changes no stored byte.

- `docs/llama-adapter.md`'s patch section now describes the v2 protocol, not
  v1, and scopes the streamed-transport section to the fixture and planning
  protocols. S033's opening no longer says "not an implemented capability".
  It says the protocol is implemented against mock workers and is not yet
  qualified live.
- New tests: a historical v1 whole-file edit still offers, accepts and
  finalizes and its review shows no replacement bytes; a saved v1 edit action
  blocks a v2 session before any POST; a linked parent directory (symlink on
  Linux, junction on Windows) never yields outside bytes. That test fails
  closed at the run's snapshot check, so it shows the read is refused. It does
  not isolate the per-component link check. (Corrected by the audit, A6, in
  the entry below.)
- Path rules: a spelling table for `canonical_text_patch_path` found two gaps
  in the Windows reserved-name check. `CON .txt` (a space before the dot) and
  superscript-digit names were accepted. Both are now refused. This only
  tightens. A mutation check confirmed that the table fails without the fix.
  (The first version of this entry called the table "exhaustive". It is not.)
- Gates on the final tree: Windows 259 passed, Docker Linux 264 passed, none
  failed, 10 ignored. The independent Opus : high audit is still pending, and
  so is Chunk 7.

The Chunk 6 independent audit (Opus : high, 2026-09-25) re-observed both gates
and found no safety defect. It did find four required fixes and four smaller
items. All eight are applied:
- **A1.** The session's refusal of a run holding a saved v1 edit *request*
  had no test. With the check deleted, every test still passed.
  `a_saved_v1_edit_request_alone_blocks_a_v2_session_before_any_inference`
  now covers it.
- **A2.** `docs/llama-adapter.md` had repeated "a failure before the started
  marker cancels", the overclaim that review R4-2 corrected, and had dropped
  the Windows archive-attribute exception. Both now follow S033.
- **A3.** `task-edit-review` told the retained r4 run "No admitted edit has
  been attempted", although r4's v1 request reached the worker. It now reports
  recorded v1 edit requests, using the new `TaskReader::legacy_edit_requests`.
- **A4.** The historical v1 test's fixture did not match what the removed
  executor stored. The test now writes the executor's exact rows: the grant,
  the v1 request and its reply, action ID, grant revision 2, the pre-edit
  input hash and the artifact. Its only reduction is that the saved request
  keeps just its `protocol` field. The action-only refusal test keeps a
  deliberately synthetic fixture, labelled as such. A real v1 action always
  has its request, and the request check refuses first.
- **A5.** `CONIN$` and `CONOUT$` are now refused too. The reserved-name comment
  now says the list is defensive and records what was observed on Windows 11.
- **A6.** The linked-parent test now asserts the link refusal and also uses a
  byte-identical outside copy, which only the link check can refuse. A
  linked-workspace-root test was added. The refusal comes from the
  per-component check that runs inside the snapshot capture. The edit path's
  own copy of that check is still untested in isolation. It is redundant with
  the handle-identity and admitted-hash checks.
- **A7 and A8.** The v1-action test now asserts its exact message, and S033's
  opening dates are corrected.

Gates on the fixed tree are recorded in the plan's audit section. The audit's
focused re-check (Opus : high, a separate session, 2026-09-25) passed. It
found two Low items, both fixed: `task-edit-review`'s v1 sentence
now says only `run-status` lists the requests (RC-1), and the v1 request
fixture now pauses the run as the removed executor did (RC-2). The operator
accepted Chunk 6 on 2026-09-25.

Chunk 7 (2026-09-25) repeated r4's objective, constraints and profile against
the live Qwen3.5-4B worker
([record](manual-qualifications/emcp-2026-09-25-chunk7.md)). Attempt 7g
completed the whole v2 workflow:
- planning, the grant for `AGENTS.md` only, and three bounded reads;
- a compact patch of 1,367 bytes and 221 of 512 output tokens, applied
  exactly;
- re-admission, and the full emCP check passing (19 files, 125 tests);
- the offer, and an explicit operator reject.

The patch invents `npm run test:mcp` and misses the guidance the objective
targets. A separate Opus : high evidence review found no protocol defect and
no raised limit or weakened gate. It put the wrong patch down to the edit
context and the model's read strategy.

On the way, the verification plan needed fixes: `node.exe` instead of
`npm.cmd`, and an environment with `ComSpec`, `PATH`, `TEMP` and `TMP`. The
fixed plan is "plan `e`". The original plan file is known-broken. The work
also exposed two executor follow-ups: the verbatim `\\?\` working directory,
and the `unknown` status label.

The operator amended Chunk 7 criterion 6 and S033 intended outcome 8 so that
they require an explicit decision on the merits rather than acceptance. On
that basis, S033 is closed (2026-09-25; see S033's closure clarification).
This qualifies the protocol, not task success. Still open:
- Chunk 7 step 9, the Windows Terminal and native Linux manual-usability
  records;
- the executor and plan-file follow-ups;
- an edit-context-sufficiency increment and a comparable live run.

Closing S033 does not close Milestone 3 (see the scope check below).

Executor follow-ups from Chunk 7 (K3, 2026-09-25). The operator has not yet
accepted the review-fix commit. The first K3 commit (`8655250`) received an
independent Opus : high review; its findings and their disposition follow the
list.
- **K3a.** Task preflight runs the launch checks that need neither a grant nor
  the working directory (`validate_process_spec_static`: platform, limits,
  absolute executable path, the Windows `.exe` rule, executable hash, argument
  and environment bounds) on every unwaived check. A plan that names `npm.cmd`
  is refused before admission and before any model turn. This is the first
  admission only: re-admission of an existing task does not repeat it, and a
  plan whose executable later changes is refused when a check is prepared. It is
  not a proof that a plan can launch: a `.exe` that is not a valid executable
  passes it.
- **K3a.** The working directory is deliberately not checked at preflight. An
  earlier check may create it (`cmake -B build`, then a check that runs in
  `build`), and each check still requires it when that check is prepared.
- **K3a.** `task-verify-status` reports `not_prepared` for a check bound to the
  admission whose preparation was refused, and `not_started` for a prepared
  action that never started. `unknown` is reserved for a started or unknown
  action with no receipt. (Before the review fix a prepared action was
  reported as `unknown`.)
- **K3b.** On Windows the child's working directory is passed without the
  `\\?\` prefix when the plain form names the same directory; identities are
  unchanged.
- **K3c.** The launch-boundary repeat of the checks, after the durable started
  marker, distinguishes an absence from every other refusal. A missing
  executable or a missing working directory means no launch was attempted; it
  is recorded as a `Failed` observation with `SpawnFailed` and a bounded
  `refused before launch: …` reason, not an unknown effect. Every other
  refusal stays an error, so the run pauses with the action started and
  reopening makes it unknown: a grant, authorization or declared-input
  mismatch, a changed executable hash, a symlink, reparse point or escape in
  the working directory, an exists-but-not-a-directory working directory, and
  an invalid specification. Those may be tampering, not absence.
- **K3c, scope for admitted checks.** A verification check's snapshot covers
  its executable identity, and `dispatch_inner` revalidates that snapshot
  after the started marker and before execution. So for an admitted check a
  changed or missing executable fails there and still leaves the action
  unknown. Only a working directory, which the snapshot does not cover,
  reaches the recorded-failure path. This deliberately covers less than the
  original plan's "any pre-spawn re-validation failure".
- **K3c tests** (`tests/process_execution.rs`). Absence is recorded (direct and
  through the controller, with the reserved active time refunded and the
  failure kept across reopen and replay); a changed executable, a working
  directory swapped for a link, and input drift stay errors, and the changed
  executable leaves the action unknown on reopen. Mutating the
  absence-versus-refusal classification either way fails them. A unit test in
  `src/main.rs` pins every status label, and `tests/task_intake.rs` covers the
  preflight scope.
- **Review of `8655250` (Opus : high, fresh session, read-only).** No defect in
  the started/unknown/replay-block guarantees, no raised limit and no weakened
  freshness gate. Findings and outcome: (1) docs overclaimed a changed
  executable being recorded as a failure for admitted checks: fixed by the
  scope paragraph above. (2) preflight refused plans whose working directory a
  prior check creates: fixed, the directory is no longer checked. (3) demoting
  a changed executable or a link in the working directory contradicted the
  integrity rationale: fixed by narrowing K3c to absences (option A). (4) every
  working-directory error was labelled "must exist": fixed, the label applies
  only to a missing path. (5) `unknown` for a prepared action: fixed
  (`not_started`). (6) re-admission does not repeat the preflight gate:
  documented, not changed. Not applied: checking cancellation before the
  launch checks, and re-running the launch checks at re-admission.
- **K3d.** The Windows environment baseline (`SystemRoot`, `ComSpec`, `PATH`,
  `TEMP`/`TMP` outside the workspace, `node.exe` with `npm-cli.js`) was already in
  the manual qualification guide. That guide is a local file under the ignored
  `docs/manual-qualifications` directory and has never been committed, so this
  was not a repository record. The baseline is now also recorded in the tracked
  [plan v2 record](manual-qualifications/emcp-plan-v2.md).
- Gates on the final tree, after the review fixes: Windows 272 passed, 0 failed,
  10 ignored; Docker Linux (pinned Rust 1.98 / Python 3.14.7 image) 273 passed,
  0 failed, 10 ignored; formatting and warnings-denied Clippy pass on both.
  Clippy first rejected the new `src/main.rs` unit-test module for sitting before
  other items; it now sits at the end of the file. An earlier Windows test run of
  the first K3 commit failed to build because of corrupted `tokio`, `tokio-util`,
  `icu_provider`, `zerotrie` and `cortex-shuttle` artifacts in `target`, not a code
  defect; only those packages were cleaned. Logs are in ignored
  `.shuttle/k3fix2-windows.log` and `.shuttle/k3fix2-docker.log`.
- S034 (admitted edit context sufficiency: read-only reference files, a
  labelled plan summary and a revision-2 context) was drafted, independently
  reviewed twice and accepted by the operator on 2026-09-26. K5b is
  implemented: context revision 2 (reference files, a labelled plan summary and a
  revision-specific prompt with a permitted-path `enum` on the patch tool's paths), the
  fit-at-open candidate loop through a provider factory, the canonical-encoding
  check at load, and a synthetic revision-1 golden that pins the old bytes.
  Windows 299 and Docker Linux 300 tests pass (none failed, 10 ignored on each)
  with formatting and warnings-denied Clippy clean. An independent review found no
  limit raised, no authority widened and no stored revision-1 identity changed,
  and its fixes are applied. The live probe (operator-authorized, standalone
  requests, nothing journaled) found the path `enum` accepted in every schema but
  enforced unreliably for read and find (5 of 10 explicit attempts) and reliably for
  the patch tool (4 of 4), with 3 of about 27 enum requests running to the token
  limit with no tool call. The operator chose to keep the `enum` on the patch tool
  only, and K5b ships it there (a recorded amendment of S034 Decision 3).
  K5c is implemented too: `task-edit-review`, the terminal task view and the
  offer review show the context revision, the reference files, the omitted count
  and the plan-summary state (the offer line is bound to the session that prepared
  its edit), with revision-1 and older-journal output unchanged. Windows 309 passed and Docker Linux 310 passed, with none failed and 10 ignored on each,
  and a second independent review of the whole increment (K5d) found no read of the
  workspace, model call or authority in the review and no change to revision-1
  output; its findings are applied. The operator accepted the implementation on
  2026-09-26. Not done: K6's live run, an end-to-end revision-1 session golden from
  a pre-change harness, and proof of composed-size monotonicity and of the adapter
  POST and intent size classifications at open; acceptance does not change what
  S034's obligations table lists as unproven, which the clarifications at the end
  of S034 record.
- K4 (2026-09-26): emCP verification plan v2 written and validated by a scratch
  intake and preflight, with no emCP file touched. It replaces the known-broken
  plan file and plan `e`, which named `C:\dev\agentic\emCP` (no longer present),
  drops the `--prefix` workaround, declares four inputs sized to the preview pool
  and the edit-context budget, and adds a read-only `doc-scripts-exist` check. See
  [the record](manual-qualifications/emcp-plan-v2.md). The operator ran the
  model-free baseline against emCP the same day (`.shuttle\k4-baseline`): both
  checks passed, the emCP tree was clean before and after, and the plan revision
  and preflight snapshot matched the scratch validation. It also confirmed the
  K3b working-directory change live: `full-check` passed with no `--prefix`. The
  test and reference counts and the duration were not captured in the receipts.
  One deviation is recorded: the baseline intake holds a hand-typed constraint
  ("agents.MD") that differs from 7g's; it affects nothing in that run, and K6
  must use a new state directory and the exact 7g text, checked by hash before
  its intake.
- K4 is complete, and S034's implementation (K5) is done and accepted. Still open
  from Chunk 7: K6's live run and step 9's manual terminal records. The K6 runbook
  and a read-only transcript dump script (`k6-transcript-dump.py`, which checks
  whether the target lines and the `package.json` scripts reached the model) are in
  [the plan-v2 record](manual-qualifications/emcp-plan-v2.md). The optional
  workspace-pollution evidence stays deferred.

## Milestone 3 scope check (2026-09-21)

The master plan (`docs/.shuttle-priv/dedicated-harness-plan.md`, moved here
from CortexWeave on 2026-09-21 and gitignored) files the admitted-edit v2
protocol under Milestone 3, "Context and repository history," whose full
deliverable list is bounded request composition, conservative refresh policy,
native repository membership, eligible sibling retrieval, and separate
history/hydration inventories, with exit evidence including sibling positive
and isolation/lifecycle/removal negative cases.

The Chunk 0-7 plan only covers the first two of those five deliverables. Its
own scope section is explicit: existing declared regular UTF-8 files in the
single currently-admitted workspace. It does not mention repository
membership, sibling retrieval, or hydration inventories anywhere, and no
chunk in it targets the master plan's §6.1 mixed-packet hydration contract,
`HarnessHydrationRequest::from_context` origin separation, or sibling-worktree
test scenarios. Completing Chunk 7 will not by itself satisfy Milestone 3's
exit evidence. This entry exists because the gap was previously silent: the
"Repository-scoped historical Experience... stay in their planned later
milestones" note above (2026-09-13, predating this entire increment) reads as
still current but does not say whether that work is expected inside Milestone
3 or after it. That ambiguity is unresolved as of this entry.

A chunk plan for the remaining three deliverables (Milestone 3b: repository
membership, sibling retrieval, hydration inventories) was later drafted at
[`docs/plans/WHO-COUNTS-AS-THE-SAME-REPOSITORY.md`](plans/WHO-COUNTS-AS-THE-SAME-REPOSITORY.md),
Chunks 8-12, continuing the Chunk 0-7 numbering. It answers this entry's
question by treating 3b as a separate delivery sequence after 3a, but that is
the plan's own framing, not a resolution the operator has confirmed. Drafting
the plan is not starting it: no Chunk 8-12 work has begun.
