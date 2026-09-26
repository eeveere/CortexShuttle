# Terminal qualification

## Durable task view (2026-09-09)

`shuttle task-view --state-dir PATH` is the first terminal view wired to an
actual journal. It renders the persisted objective, phase/reason, ordered action
states, provider count, active/process budgets, native-delivery backlog, stall
reason and acceptance status. Its headless sibling,
`shuttle task-view --state-dir PATH --json`, serializes those same fields for
automation. Neither form executes a command, grants permission, accepts an offer
or finalizes a run; those effects remain on the existing explicit commands.

The view reads one journal snapshot before opening the terminal and does not claim
live refresh. It may become stale while displayed; restart it to inspect later
state. The task view restores terminal modes through the same guard as the preview.
On Windows, the command dispatcher is boxed before entering command-specific async
work; this avoids a main-thread stack overflow observed when opening SQLite/native
futures from small CLI commands.

Focused Windows terminal-layout/PTY tests cover the rendered durable and headless
projection. The finalized live repair journal was exercised through both
`task-view --json` and `run-status`; it displayed five durable actions, four model
requests, no delivery backlog and the accepted offer. Final locked qualification
passed 104 Windows tests and 107 Docker Linux tests, with eight standalone helper
fixtures ignored on each platform; formatting and warnings-denied Clippy passed.

The preview remains an interaction study: seven sample states, bounded editable
input and explicit exit keys. Enter cannot execute a command or record acceptance.
Real task review and decisions continue to use the separate explicit CLI workflow.

## Interactive task workspace (2026-09-09)

`shuttle task --state-dir PATH` opens an editable, journal-backed task workspace.
It renders the same persisted task projection as `task-view`, with scrollable
history and a multiline direction field that follows its newest wrapped lines.
Enter inserts a line; pasted text is normalized and bounded to 4,096 UTF-8 bytes.
Up/Down and PageUp/PageDown scroll the durable action history.
Ctrl+R leaves the terminal and prints the complete existing saved acceptance
review for the offer currently bound to the run. It cannot accept the offer;
the existing review operation revalidates evidence, can persist staleness, and
accounts for its active time. Acceptance and finalization retain their separate
explicit commands.

The workspace does not turn ordinary text into a model request. Its original
direction control is available for a run paused by the bounded-stall policy: Ctrl+S opens a
review prompt, and a second Ctrl+S records that exact caller direction through the
existing `resume_stall` journal operation. Esc cancels the review; existing time
budgets, evidence history, receipts, stale status, delivery backlog checks and
unknown-completion replay block stay in force. The terminal restores before the
journal is reopened to write the direction. A subsequent screen reflects the
new durable state. While open, the workspace rereads its durable projection every
half second; a failed refresh is shown as unavailable and never replaced by
invented state.

Focused layout coverage checks multiline visibility, history navigation, absence
of permission controls, submission confirmation and the fact that non-stalled
runs retain their drafts without recording a direction. The workspace is not a
general task-creation, model-request, process, permission, acceptance or
finalization interface. The subsequent scripted-task start control is described
under Task lifecycle and observation below.

Final qualification passed 106 Windows tests and 109 Docker Linux tests, with
eight standalone helper fixtures ignored on each platform. Formatting and
warnings-denied Clippy passed with locked dependencies. Linux used the pinned
Rust/Python 3.14.7 qualification image in `tests/Dockerfile.qualification`; the
generic Rust image cannot meet the unittest producer's exact runtime identity.
Logs are retained in ignored `.shuttle/workspace-windows-*.log` and
`.shuttle/workspace-linux.log`.

## End-to-end admitted-task workspace (2026-09-13)

`shuttle task` is now the terminal client for the bounded admitted-task flow. It
can create the initial intake when launched with `--state-dir`, `--workspace`,
`--objective`, and `--plan`; a subsequent launch opens the same durable task.
`--profile` is required only for the explicitly confirmed local-model plan/edit
operations, while `--approve-host-execution` is required only for explicitly
confirmed verification. Neither value is accepted as free-form terminal input or
saved as ambient authority.

The workspace projects saved intake/preflight/admission state, pending or unknown
work, actions, suite-evidence status, task-offer/decision status, and bounded
factual review material. Its journal-derived direction is advisory only. The
operator presses the same shortcut twice to request an operation: Ctrl+P
preflight, Ctrl+A admit, Ctrl+L plan, Ctrl+W grant, Ctrl+E edit, Ctrl+V verify,
Ctrl+O offer, Ctrl+D re-admit, Ctrl+Y accept, Ctrl+N reject, or Ctrl+F resume
finalization. Esc cancels the pending confirmation. Ctrl+R leaves the terminal to
show the saved offer review.

The terminal restores before an operation runs. The existing workflow then owns
all freshness checks, write-permission validation, started/unknown recovery,
evidence checks, immutable decision recording and ordered native delivery. A
screen redraw, key repeat, or stale cached projection cannot grant authority or
manufacture a pass, acceptance, or finalization. Rejection records no native
finalization. Full review details remain available through `task-offer-review`;
the workspace renders the admitted edit as bounded, escaped review lines (file
and hunk counts, hashes, and the exact compact hunks, at most 1,024 bytes per
side and 160 lines, with an explicit "not shown" marker for anything cut)
rather than whole files or unbounded artifacts. `task-edit-review` prints the
same review, and `--json` adds every hunk in full.

Focused terminal layout, interaction, task-workspace and planning coverage passed
on Windows. The final read-only Docker Linux qualification completed with the full
locked test suite, doc tests, formatter, and warnings-denied Clippy passing.
The full operator procedure is in
[manual admitted-task qualification](manual-admitted-task-qualification.md).

## Automated boundary

### Task lifecycle and observation

`shuttle task-new --root .shuttle` creates a fresh isolated scripted fixture and
its durable run, without executing model/tool actions. `shuttle task` lists saved
tasks under `.shuttle`; Up/Down selects and Enter opens. `--root PATH` changes the
search directory, and `--state-dir PATH` directly selects an existing task.
Discovery lists at most 256 immediate task directories, without migrating or
recovering them. Incompatible/incomplete journals appear unavailable.

For newly created scripted tasks, Ctrl+G shows the exact fixture-write scope and
a second Ctrl+G starts/resumes the existing controller workflow. The terminal is
restored during execution and reopened on completion. The `demo` CLI shares this
same library path. Only tasks with the persisted `scripted_fixture_v1` binding are
admitted by this control; older/provider-backed journals remain inspectable but
cannot silently become scripted runs. The run identity, workflow and recovery
state are rechecked after acquiring the exclusive controller lock. Missing task
files are not recreated by this operation. Completion is the development review
checkpoint, not user acceptance or a qualified verification offer.

`task` and `task-view` now use a separate SQLite read-only observer. It takes no
controller lock, performs no migrations/recovery, and samples all displayed data
in one read transaction. Thus a running controller can advance while a viewer is
open, and viewing STARTED work never changes it to UNKNOWN. Pending actions,
provider requests and native deliveries are shown before history. Half-second
polling is a target interval, not a latency guarantee. A failed read labels the
cached state unavailable and disables commands; a changed projection invalidates
pending confirmations. Repeat key events cannot confirm start/resumption.

The observer reports saved receipt/offer state, not continuous filesystem
verification. Ctrl+R invokes the existing revalidating review after leaving the
observer. That operation requires exclusive ownership and can report a busy owner.
No arbitrary prompt-to-code workflow, live token stream, background runner,
interrupt control, persistent draft, or cursor/grapheme editor is provided.
Task initialization across fixture files and native/journal databases is not
atomic: interrupted creation may leave an inspectable incomplete directory;
creation never overwrites or silently retries it. Ordinary task-picker symlinks
are skipped, but discovery is not a hostile-filesystem security boundary.

Tests cover concurrent owner/observer/CLI reads, durable prepared/started/unknown
state across recovery, idle creation, exact-run admission, shared-workflow replay,
missing journal behavior, blocked unknown work, confirmation invalidation and
repeat keys. ConPTY/PTY tests exercise live redraw, explicit start confirmation,
task selection and restoration of terminal modes.

Final locked qualification passed 112 Windows tests and 115 Docker Linux tests,
with eight standalone helper fixtures ignored on each platform. Formatting and
warnings-denied Clippy passed. Linux used the pinned Rust/Python 3.14.7 image with
a read-only source mount and separate Cargo/target volumes. Evidence is retained
in ignored `.shuttle/lifecycle-final-windows.log` and
`.shuttle/lifecycle-final-linux.log`. The corrected task-picker redraw assertion
also passed focused ConPTY and Linux PTY reruns.

`terminal_interaction` launches the actual UI code in Windows ConPTY and Linux
`openpty` children. Tests send UTF-8 text, backspace, Tab/BackTab, Esc and Ctrl+C,
resize the terminal and assert a rendered frame uses the new dimensions. A child
compares terminal modes before and after normal return, injected error, unwinding
panic, prior raw mode and customized input mode. Linux also tests bracketed paste
with Unicode and newlines. Parent output capture is bounded and children have exit
deadlines. Windows startup explicitly supplies null standard handles so the child
uses its pseudoconsole instead of the parent test runner's redirected pipes.

`terminal_layout` covers all seven states, narrow rendering, Unicode-safe bounded
input, control filtering, key repeat/release behavior, newline keys and the absence
of implicit execution or acceptance. Redraw tests validate dimensions and input
state, rather than assuming a terminal's optimized output repeats complete strings.

The UI restores raw/input/output modes, leaves its alternate screen and shows the
cursor on ordinary return, returned errors and unwinding. It assumes entry from
the normal screen with a visible cursor; nested alternate screens are unsupported.
Output failures may prevent escape-sequence restoration, forced termination cannot
run cleanup, and a panic hook may print before unwinding restores the screen.
The preview owns terminal use exclusively for its lifetime.

Input is capped at 8,192 UTF-8 bytes. Backspace removes one Unicode scalar, not a
grapheme cluster. The two-row message viewport follows the end of the input,
including trailing empty lines, and recomputes scrolling after wrapping, resize
and backspace. Manual navigation through earlier input is not implemented yet.
Linux bracketed paste normalizes line endings and removes control
characters except tab/newline. Crossterm's Windows console reader produces key
events, so bracketed paste is not enabled there: ordinary printable paste works,
but pasted tabs/newlines may act like keys. Shift+Enter depends on terminal keyboard
support; Ctrl+J supplies a fallback. IME, clipboard integration, accessibility,
complex grapheme editing and every terminal emulator remain unqualified. The input
buffer is bounded; the terminal library may allocate while decoding paste first.

## General-task intake boundary

`task-intake` creates a task directory only after validating its existing workspace,
objective, constraints and bounded verification-plan JSON. It persists the full
plan and exact revision through migration 0009, but does not create a run, start a
controller, contact a model, invoke a command, or offer acceptance. The observer
renders such state as `intake` and disables the scripted start control. Focused
Windows/Linux tests verify restart-visible revision binding, no fixture creation,
invalid input rejection and refusal by the scripted fixture workflow. The original
intake command remains available for automation. The terminal now provides bounded
intake authoring and the subsequent supervised transitions described above; it is
still not a filesystem watcher, arbitrary repository runner, or general permission
surface.

Final locked qualification for this increment passed 115 Windows tests and 118
Docker Linux tests, with eight standalone helper fixtures ignored on each platform.
Formatting and warnings-denied Clippy passed in both environments.

## Intake-preflight staleness

A changed-input `task-preflight` keeps the original snapshot and marks its binding
stale as it saves the new one. The task projection reports retained stale-preflight
history. This status is monotonic: restoring files does not make an earlier
observation current again. Detection occurs during explicit preflight refresh, not
through a filesystem watcher, and it still does not admit or run work.

Final locked qualification for this increment passed 116 Windows tests and 119
Docker Linux tests, with eight standalone helper fixtures ignored on each platform.
Formatting and warnings-denied Clippy passed in both environments.

## General-workflow admission boundary

`task-admit` compares a new declared-input capture with the latest fresh
preflight, then saves an immutable general-verification contract if they match.
It rejects changed, missing or stale evidence and remains a non-runnable state:
the task view says `NO EXECUTOR admitted`. Focused tests cover changed-input
rejection, one-time admission, blocked later preflight, and refusal by the
scripted fixture workflow.

## General-task preflight boundary

`task-preflight` captures and persists a new bounded source snapshot from the
saved intake's exact plan and workspace. Migration 0010 binds it immutably to the
intake and plan revision; later declared-input edits produce another retained
snapshot. The command does not run a check or admit a workflow, and it rejects a
task that already owns a run. Focused tests cover wrong-identity rejection,
restart-visible snapshot display, changed-source snapshots and no scripted-workflow
adoption. It is not a source watcher, permission flow, controller, or executor.

Final locked qualification for this increment passed 116 Windows tests and 119
Docker Linux tests, with eight standalone helper fixtures ignored on each platform.
Formatting and warnings-denied Clippy passed in both environments.

## Human review

2026-09-08: the user reviewed the preview, reported the multiline input visibility
issue and approved the scrolling fix ("looks great"). They then concluded this
review round with no other issues reported, qualified as "for now at least".
This records provisional approval of the reviewed UI, not permanent sign-off or
completion of every platform/checklist combination. Terminal versions, font,
window sizes and separate native Linux review were not specified.

The following procedure remains available for further platform qualification:

Run `cargo run --locked -- preview` in Windows Terminal and in a native Linux
terminal. Repeat with `--reduced-color`. Record terminal/OS version, font and scale,
window size, date and reviewer with any observed defect; do not infer sign-off from
the automated checks or an SVG export.

1. Inspect all seven states at 80 x 24 and 120 x 36, and the resize message below
   42 x 16. Check readability, clipping, focus, contrast and modal text.
2. Type and paste a short Unicode message, use backspace, test the newline keys,
   and cycle states in both directions. Check the host-specific paste limits above.
3. Exit with Esc and Ctrl+C in separate runs. Confirm the shell screen, cursor,
   echo and subsequent typing behave normally.
4. Confirm the permission and acceptance displays clearly look illustrative and
   that Enter does not activate them. Record usability acceptance explicitly.

Automation does not establish visual approval. Platform-specific coverage beyond
the user's reported review remains unrecorded.

2026-09-09: the user manually created and ran two independent scripted tasks through
the new task picker and explicit Ctrl+G confirmation, then reviewed and approved
the results. Each task recorded one four-action scripted workflow and reached
`awaiting_review` with no pending work. This approves the reviewed lifecycle flow;
it does not approve general repository task intake, arbitrary model execution, or
the unqualified terminal/accessibility cases recorded above.

## Reproduce the development checks

Shuttle pins CortexWeave to published revision
`754126bfc2efd6330253826c89922148d33d9915`. Cargo.lock records the same Git source
and commit. There is no local path override, and a sibling CortexWeave checkout is
not required. Cargo needs network access on the first fetch or an existing cache.

On Windows run formatting, locked tests and warnings-denied Clippy in Shuttle
under the normal user account. Docker Linux uses a read-only Shuttle source mount:

```powershell
docker build -f tests/Dockerfile.qualification -t shuttle-qualification:local .
docker run --rm --init --cpus 2 --memory 6g `
  --mount type=bind,source=C:/den/CortexShuttle,target=/src,readonly `
  --mount type=volume,source=shuttle-linux-cargo,target=/usr/local/cargo `
  --mount type=volume,source=shuttle-linux-target,target=/target `
  -e CARGO_TARGET_DIR=/target -w /src shuttle-qualification:local `
  sh -c 'cargo fmt --all -- --check && cargo test --locked && cargo clippy --locked --all-targets -- -D warnings'
```

Adjust the host source path on another machine. Docker PTY tests
exercise the Linux terminal driver and application, not a desktop Linux emulator.
