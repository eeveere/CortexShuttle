# DO WE REALLY HAVE TO REPLACE THE WHOLE FILE?

## Bounded transactional text-patch protocol plan

Status: Step 0 reassessment done 2026-09-18 (see "Step 0 result"). Chunks 0, 1,
2a and 2b are complete; Chunk 2b's independent adversarial review ran 2026-09-20
and its required fixes F1–F3 are applied (recorded in
`docs/implementation-status.md` and S033's 2026-09-20 clarification), so 2b is
accepted. Chunk 3 (v2 wire protocol) was implemented 2026-09-24, and its review
fixes are in commit `d0bd549` (see "Chunk 3 result" and the note in "Chunk 4
split"). The 2026-09-24 reconciliation ran the first Windows and Docker Linux
full-suite gates for Chunks 2a–3; both pass (see "Reconciliation result").
Chunk 4 began 2026-09-24 and is split into 4a–4d (see "Chunk 4 split and
results"). Its fresh-task adversarial Opus : high review ran 2026-09-24 (see
"Chunk 4 adversarial review"). All four review fixes (R4-1..R4-4) are
applied. The focused re-review ran the same day, in the same session that
applied the fixes, so it is not independent. It found and fixed one gap in
R4-3 (RR-1). Both gates pass on the final tree, apart from the known
`llama_adapter` flake on Windows. Chunk 4 was accepted by the operator on
2026-09-24 (see "Chunk 4 acceptance"). Chunk 5 (projections) was implemented the same day and
gated on both platforms. Its Sonnet : high review ran the same day, and fixes
R5-1..R5-5 are applied and gated (see "Chunk 5 review"). The operator accepted
Chunk 5 on 2026-09-25 (see "Chunk 5 acceptance"). Chunk 6 (consolidation) was implemented
2026-09-25. Its independent Opus : high audit ran the same day and found
four small required fixes (A1–A4) and four smaller items (A5–A8). All eight
are applied and gated on both platforms (Windows 261, Docker Linux 266, none
failed). A focused re-check in a fresh session is pending (see "Chunk 6
independent audit"). This document does not authorize an implementation or
alter the failed task state.

The terminal lifecycle/flashing issue is deliberately outside this plan.

## Step 0 — Re-evaluate the plan and rewrite Claude model:effort guidance

Before resuming Chunk 2 or starting any later chunk, Claude must reassess this
plan against the current repository and working tree. The reassessment is a
required planning gate, not a request to repeat completed work.

Claude must:

1. inspect the current implementation and identify which deliverables are complete,
   partially implemented, unverified, or stale;
2. verify that S033, the retained r4 evidence, and the current code still agree on
   bounds, permissions, durability, freshness, replay, and compatibility;
3. re-check chunk ordering and dependencies, moving or splitting a chunk when the
   current code exposes a safer boundary;
4. record a replacement model:effort table using Claude's available model and effort
   settings; and
5. preserve every safety invariant and exit gate unless a written contract revision
   is reviewed and accepted first.

Use task shape to choose Claude effort rather than copying the historical Codex
model names below:

| Work shape | Claude guidance |
| --- | --- |
| Contract, crash/replay, permission, freshness, or cross-platform safety reasoning | Highest available reasoning effort; require an independent adversarial review. |
| Rust integration, durable transactions, and bounded security tests | High effort, with focused tests and a separate review when the change crosses a durable boundary. |
| Projections, documentation, fixtures, and narrow compatibility updates | Medium effort after semantics are frozen; keep the exact exit gate visible. |
| Formatting, targeted static checks, and mechanical maintenance | Low effort is sufficient when no behavior or contract changes. |

Do not map `gpt-6-astra`, `gpt-5.6-terra`, or `gpt-5.6-sol` mechanically onto
Claude. The model assignments in the chunk history are retained as an audit trail
for completed Codex work; the new Claude table supersedes them for all future work.
The reassessment must end with an explicit next chunk, its context pack, its
Claude model:effort choice, its reviewer, and its exit gate.

## Step 0 result — Claude reassessment (2026-09-18)

Performed by Claude (Opus, high effort) against the working tree as last modified
2026-09-16 21:49. No production code, worker settings, or task state changed. The
repository has no commits, so "diff from predecessor" is not yet available; the
first commit should follow the build gate below.

### Deliverable status

| Item | Status | Evidence |
| --- | --- | --- |
| Chunk 0 — S033 and frozen r4 | **Complete** | S033 accepted in `docs/decisions.md`. The three r4 source files still match `.shuttle/chunk-0-r4-source-hashes.json` (SHA-256 rechecked 2026-09-18). |
| Chunk 1 — pure planner | **Complete, exit gate passed 2026-09-18** | The planner, v2 types and bound constants match S033; all 12 table-driven tests in `tests/workspace_text_patch.rs` pass. See "Chunk 2a result". |
| Chunk 2 — pure read/find (`src/edit_session/text.rs`) | **Implemented, unverified** | Matches S033 line/CRLF/lone-CR/EOF/find-overlap/ninth-match/budget rules. 12 unit tests exist and have never been compiled or run. |
| Chunk 2 — session ledger (migration `0022`, `src/edit_session.rs`, `requests.rs`, `journal.rs` recovery) | **Complete and verified 2026-09-18** | Turn reservation, observation commit, failure/discard commit, v1-evidence refusal and unknown→closed recovery exist, compile, and are covered by 13 journal-level tests in `tests/edit_session.rs`. G1 was a misdiagnosis (see below); G2 decided, G3/G5 fixed. |
| Chunk 2 — journal-level safety tests | **Complete 2026-09-18** | 13 tests in `tests/edit_session.rs` cover changed-input (target and non-target), ungranted, traversal/absolute, non-UTF-8, hard link, oversized range, oversized artifacts, exhausted turns, settled-request replay, interrupted-turn recovery and restart/replay. |
| Chunk 2 exit gate (read beyond the 1 KiB preview) | **Demonstrated 2026-09-18** | `read_reaches_beyond_the_initial_preview_without_any_write_capability` proves an exact excerpt past the truncated preview, with no write or command capability and no action. No v2 wire adapter exists yet (Chunk 3), as expected. |
| Controller dispatch of v2 decisions | Correctly absent | `controller.rs` rejects `AdmittedTextRead`/`AdmittedTextPatch`. Orchestration belongs to Chunk 4, not Chunk 2. |

### Contract agreement check (S033 vs code)

| Area | Result |
| --- | --- |
| Bounds | Agree: 4 reads, 2,048/6,144 excerpt bytes, 16 KiB observation, 5 turns with a patch-only fifth, 128 lines, 256-byte literal, 8 matches. They are also enforced by SQL `CHECK`s. |
| Permissions / paths | Agree: canonical path, exact grant membership, declared-file lookup, per-component symlink/reparse check, canonical-root containment, link count of 1, handle identity rechecked after read. |
| Durability / replay | Agree: turn and request reserved in one transaction; observation, result, artifacts and counters committed together; triggers forbid refunds and edits; restart turns started into unknown and closes the session. |
| Compatibility | Agree: sessions refuse runs with v1 edit evidence; provider/purpose strings match S033; request reuse compares the full intent bytes. |
| **Freshness** | **Agree. G1 was a misdiagnosis — corrected 2026-09-18 in Chunk 2b.** The original finding claimed `fresh_edit_session` checked only DB-level staleness. It does not: its first call is `current_write_permission_view`, which computes `unusable_reason` from a live `SourceSnapshot::capture` compared against `admission.preflight_snapshot` (`src/workspace.rs` ~L1646-L1673), and `fresh_edit_session` requires that reason to be `None`. Because `finish_admitted_text_read` calls `fresh_edit_session` both before and after the read, S033's "checked before and after access" was already satisfied, for every declared input rather than only the read target. See the Chunk 2b result. |

Lower-severity notes (fix during Chunk 2 or record as a clarification):

- **G2 — closure breadth.** Any error in `prepare_admitted_edit_turn` closes the session permanently, including an unrelated pending request or history overflow, where S033 says "pause". This is stricter, not weaker. Record it as the intended behavior in S033, or narrow it.
- **G3 — early bail.** In `finish_admitted_text_read`, a non-started request or over-cap artifacts returns before any failure commit. The request stays `started` until restart marks it unknown. That is contract-compatible, but the run should pause immediately.
- **G4 — discarded prepared state.** `close_edit_session` leaves discarded requests as `state='prepared', applied=1`. Confirm that projections and readers render this (Chunk 5 surface).
- **G5 — magic number.** `turn < 4` is hard-coded. Use `MAX_EDIT_TURNS - 1`.
- **Carried to Chunk 3:** the POST `body + max_tokens + 4096 <= n_ctx` check, and duplicate-key rejection for the internally tagged `TextReadOperation`.

### Ordering change

Chunk 2 is split so the unverified foundation is proven before more code lands on it:

- **Chunk 2a — build gate. Complete 2026-09-18.** A working toolchain (native
  Rust 1.98.0, not the Docker image — see "Chunk 2a result" for why) ran `cargo
  fmt --check`, `cargo check --locked --tests`, warnings-denied Clippy, `cargo
  test --locked --test workspace_text_patch`, and the `edit_session::text` unit
  tests. One mechanical Clippy fix was applied (see below); no behavior changed.
  This closed Chunk 1's runtime exit gate, and the first commit was made.
- **Chunk 2b — finish the session ledger. Complete 2026-09-18, review pending.**
  G2 decided and recorded in S033, G3 and G5 fixed, G1 found to be a
  misdiagnosis and corrected in this plan, and the journal-level tests plus the
  beyond-preview read test added. This crosses a durable boundary, so its
  separate adversarial review is required before acceptance.

Chunks 3–7 keep their order and gates.

### Claude model:effort table (supersedes the historical Codex labels)

| Chunk | Primary | Reviewer | Notes |
| --- | --- | --- | --- |
| 2a build gate | Sonnet : medium | none (mechanical) | Escalate to Opus : high if a compile fix changes behavior or a test fails for a non-trivial reason. |
| 2b session ledger | Opus : high | Opus : high, fresh task, adversarial | Freshness/replay/permission reasoning. |
| 3 v2 wire protocol | Opus : high | Sonnet : high | Hostile output, duplicate keys, byte caps. |
| 4 durable apply | Opus : high | Opus : high, fresh task, adversarial | Crash/replay boundary; highest risk. |
| 5 projections | Sonnet : medium | Sonnet : high | Semantics frozen by then. |
| 6 consolidation | Sonnet : medium | Opus : high independent audit | The audit checks docs never claim more than was observed. |
| 7 live emCP qualification | Sonnet : medium | Opus : high final evidence review | The human operator keeps all authority. |
| Formatting / lint only | Haiku or Sonnet : low | none | Only when no behavior or contract changes. |

Escalation rule: raise to the highest effort only for an unresolved safety contradiction, a cross-platform filesystem ambiguity, or replay state that focused tests cannot settle. Record the reason here.

**Toolchain policy (decided 2026-09-18, standing for the rest of this plan):**
the automation environment that runs Claude's side of these chunks cannot reach
the Docker engine on the operator's machine — it runs in a separate, isolated
Linux VM with no path to that daemon (confirmed: no `docker.sock` or named-pipe
forwarding anywhere in it, not a shared-kernel WSL setup, no `DOCKER_HOST`).
Exposing the daemon over TCP was considered and rejected: Docker's own
"expose without TLS" option only binds to the *host's* localhost, which this
automation environment cannot reach either, and the only way to make it
reachable would be binding it to `0.0.0.0` on the LAN — an unauthenticated
control plane for the whole machine, not an acceptable trade for a build gate.

So, for every remaining chunk in this plan: wherever a context pack, deliverable,
or exit gate calls for "Docker Linux" as the Linux leg of a two-platform
qualification, substitute a native Rust toolchain matching the version pinned in
`tests/Dockerfile.qualification` (currently 1.98.0), installed directly on the
machine that holds the checkout, with the target directory pointed outside the
mounted source tree (see the Chunk 2a result below for why). Record the
substitution in that chunk's result section the way Chunk 2a's result does,
rather than silently presenting it as an unmodified Docker run. If a future
chunk's qualification genuinely depends on something only the Docker image
provides — the pinned Python for the `integrations/unittest` producer, for
instance, which the native toolchain does not install — that step must fall
back to the operator running it directly (Docker Desktop is running on their
machine, just not reachable from here) rather than being skipped or faked.
This policy does not touch the historical Docker Linux results already recorded
for Chunks 0–1's predecessor work in `docs/implementation-status.md` and similar
qualification docs; those describe what was actually run at the time and stay
as-is.

**Toolchain policy revision (2026-09-24, reconciliation; the paragraphs above
are kept as the record of what was true on 2026-09-18).** The "Docker is
unreachable from automation" finding was true of the desktop app's isolated
device VM, which is where Chunks 2a, 2b and (via a cloud byte-copy) 3 were
gated. It is not true of a local Claude Code session running in PowerShell on
the operator's machine: that session reached Docker Desktop 29.7.2 through the
normal `desktop-linux` named pipe once the operator started it, with no TCP
exposure, and also has the Windows MSVC toolchain. From 2026-09-24 onward, the
Linux leg of every gate is therefore the pinned `tests/Dockerfile.qualification`
image again (historical run shape: `--init`, 2 CPUs, 6 GiB, source mounted
read-only, separate Cargo-home and target volumes), and the Windows leg runs
natively. The native-Linux substitution remains valid only as a recorded
fallback when a session genuinely cannot reach Docker, and a pinned-Python step
still must not be skipped or faked. Operator prerequisites observed on
2026-09-24: Docker Desktop must be running, and Visual Studio Build Tools (C++
workload) must be installed. They had been removed on 2026-09-14, which is why
`link.exe` was missing, and were reinstalled by the operator during
reconciliation.

Step 0 independent adversarial review: **pending** (optional before 2a, which has no behavior change; required before 2b is accepted).

## Chunk 2a result — build gate (2026-09-18)

Performed by Claude (Sonnet, medium effort), no reviewer (mechanical, no behavior
change).

**Toolchain deviation, recorded per Step 0's escalation-reason requirement:**
neither the Docker qualification image nor a typed Windows MSVC shell was
reachable from the automation context that ran this gate (no Docker CLI, no
`link.exe`, no root, and a network allowlist that at gate time blocked
`static.rust-lang.org`/`sh.rustup.rs`). Once `static.rust-lang.org` was added to
the allowlist, Rust 1.98.0 — the exact version pinned in
`tests/Dockerfile.qualification` — was installed via `rustup` directly on the
machine hosting the checkout, with `rustfmt` and `clippy` components. This is a
full, version-matched toolchain, just not the Docker container; it is recorded
here rather than silently treated as equivalent to a Docker or MSVC run. `cargo`'s
target directory was pointed outside the bind-mounted source tree, since the
mount cannot unlink build-script object files written under it (an unrelated
filesystem quirk of that mount, also seen on a stray `.git/index.lock` from an
earlier crashed process, which required `mv` instead of `rm` to clear before
`git add` could run).

Results, in exit-gate order:

- `cargo fmt --all -- --check`: initially failed (pre-existing formatting drift
  in `src/edit_session.rs`, `src/requests.rs`, `src/workspace.rs` and others, not
  touched by this chunk). Ran `cargo fmt --all` to fix it — mechanical,
  whitespace/line-wrap only, no behavior change — then the check passed clean.
- `cargo check --locked --tests`: passed clean, no errors.
- `cargo clippy --locked --all-targets -- -D warnings`: initially failed on one
  finding — `excerpt` in `src/edit_session/text.rs:349` has 8 arguments against
  Clippy's 7-argument default. Added `#[allow(clippy::too_many_arguments)]`
  directly above it rather than restructuring the function's signature, since a
  parameter-count lint is a style objection, not a defect, and reshaping the
  signature would touch every call site for a chunk whose whole point is "fix
  compile errors only, no behavior changes." Clippy then passed clean.
- `cargo test --locked --test workspace_text_patch`: 12/12 passed.
- `cargo test --locked edit_session::text::`: 12/12 passed (confirmed running
  under `Running unittests src/lib.rs`, not merely filtered out of every
  integration-test binary).

Command log: `.shuttle/chunk-2a-build-gate-native-linux.log`.

This closes Chunk 1's runtime-test exit gate: Chunk 1 (pure planner) is now fully
complete, not just implemented. The first commit was made immediately after,
containing the full pre-existing working tree (this was the repository's first
commit; per "Why this increment exists," Shuttle had no commits yet) plus the two
mechanical fixes above.

No production logic changed. No chunk 2 (session ledger) work was touched.

## Chunk 2b result — session ledger (2026-09-18)

Performed by Claude (Opus, high effort). **Independent adversarial review:
required and still pending.** Nothing here should be treated as accepted until
that review runs on a fresh task.

### G1 — misdiagnosed, not a gap

The Step 0 reassessment recorded G1 as the one place where the code disagreed
with S033: that `fresh_edit_session` checked only DB-level staleness and
`read_admitted_file` only the target file's own hash, so a change to a different
declared file would go unnoticed. That is not what the code does.
`fresh_edit_session`'s first action is `current_write_permission_view`, which
computes `unusable_reason` from a live `SourceSnapshot::capture` compared against
`admission.preflight_snapshot` (`src/workspace.rs` ~L1646-L1673), and
`fresh_edit_session` requires that reason to be `None`. `finish_admitted_text_read`
calls `fresh_edit_session` both before and after the read, so S033's "full
snapshot freshness is checked before and after access" was already satisfied, for
every declared input rather than only the read target.

This was established empirically, not by reading alone: the second
`SourceSnapshot::capture` that Step 0 prescribed was implemented first, and the
regression test written for it —
`changing_another_declared_input_fails_the_read_closed`, which changes
`src/other.rs` while leaving the read target byte-identical — fails closed with
the *pre-existing* message ("Declared inputs differ from the permission
snapshot"), proving the new capture never got the chance to fire. The redundant
capture was then removed rather than left in, because it doubled a
whole-declared-tree hash on every session boundary (up to four per read turn) for
no additional guarantee.

One binding that check genuinely could not make on its own was kept: the read
root comes from the session's frozen context while the freshness check captures
from the admission root, so the two are now compared after canonicalization. The
test suite exercises that assertion on every passing path, which is what shows it
does not false-positive on path spelling.

### G2 — decided: closure is intended, and is now recorded

`prepare_admitted_edit_turn` closes the session on any preparation failure, where
S033 said "pause". That stricter behavior is kept and is now written into S033
(`docs/decisions.md`, "Durable bounded context session"). The reasoning is
recorded there in full: most of these conditions are permanent for a given
session (history and serialized-request size only grow; a changed admission,
permission or snapshot identity can never match the frozen definition again), the
remainder are genuine anomalies for a single-task journal, and splitting failures
into "retryable" and "terminal" at a durable boundary would add exactly the
silent-misclassification surface this contract exists to avoid. Closure preserves
every observation, counter and artifact, so "do not discard observations or reset
counters" still holds. The operational cost is stated explicitly rather than left
implicit: a session burned this way is not reopened, and continuing needs a fresh
task state with a new human admission and grant.

### G3 and G5 — fixed

G3: the settled-request and oversized-artifact checks in
`finish_admitted_text_read` returned before any failure commit, leaving a
`started` request that only surfaced as `unknown` at the next restart. Both now
close the session and pause the run immediately. The started request is
deliberately left untouched, so it still becomes unknown and still never
authorizes a second POST. G5: the hard-coded `turn < 4` is now
`turn < i64::from(MAX_EDIT_TURNS - 1)`.

### Tests

`tests/edit_session.rs` is new: 13 journal-level tests that drive the durable
boundary directly and never dispatch a provider. They cover the required
changed-input (both a non-target declared file and the read target), ungranted
path, traversal and absolute paths, non-UTF-8 granted file, hard-linked target,
out-of-range reads, oversized artifacts, exhausted read budget with a patch-only
fifth turn, settled-request replay, interrupted-turn unknown-to-closed recovery,
and restart/replay proving saved observations are replayed verbatim and changed
source is never silently reread.

Chunk 2's exit gate is demonstrated by
`read_reaches_beyond_the_initial_preview_without_any_write_capability`: the
frozen projection is truncated at the 1 KiB per-file preview cap and cannot
contain the marker line, and the durable read returns an exact excerpt
containing it, with `fixture_writes` false, no process authorization, no action,
and no `action_id` on the session.

### Gate

Native Rust 1.98.0 per the standing toolchain policy. `cargo fmt --all --
--check`, `cargo check --locked --tests` and `cargo clippy --locked --all-targets
-- -D warnings` all pass. Tests: `edit_session` 13, `workspace_text_patch` 12,
lib unit tests 18, plus `task_planning` 10 and `verification` 12 (1 standalone
helper ignored) as an adjacent regression check, since they share
`current_write_permission_view`. The full locked suite remains Chunk 6's gate.
Command log: `.shuttle/chunk-2b-session-ledger-native-linux.log`. One incidental
note recorded there: a linker "signal 7 [Bus error]" during an earlier full-suite
link was session disk exhaustion, not a code fault.

Git was deliberately left to the operator; nothing was committed by this chunk.

### Next task

- **Blocking first:** Chunk 2b's independent adversarial review (Opus : high,
  fresh task). Chunk 2b is implemented and its gate passes, but the Step 0 table
  requires that review before it is accepted, and it has not run. The reviewer
  should start from the Chunk 2b result above and treat the G1 correction as the
  primary thing to attack: verify independently that
  `current_write_permission_view` really is on every `fresh_edit_session` path
  both before and after the read, and that removing the second capture lost
  nothing.
- **Next chunk:** 3 — v2 wire protocol.
- **Context pack:** this plan (Step 0, Chunk 2a and Chunk 2b results); the
  accepted protocol schema and bounds in S033, including the 2026-09-18
  clarification; `src/llama.rs` admitted planning/editing paths;
  `src/llama/stream.rs` only if streaming remains; local-model mock HTTP tests;
  `src/edit_session.rs` for the wire shape the session already expects
  (`EDIT_PROTOCOL`, the POST body bound, and the `TextReadOperation` schema).
- **Model:effort:** Opus : high primary, Sonnet : high reviewer, per the Step 0
  table. Hostile output, duplicate keys and byte caps.
- **Carried in from Step 0:** the POST `body + max_tokens + 4096 <= n_ctx` check,
  and duplicate-key rejection for the internally tagged `TextReadOperation`.
- **Exit gate:** a mock response representing the emCP paragraph edit remains
  well under the request, token, reply and transport bounds; v1 saved requests
  cannot enter the v2 decoder; fmt, check, Clippy and the focused test set pass
  on the native toolchain per the standing toolchain policy.

## Chunk 3 result — local-model v2 wire protocol (2026-09-24)

Performed by Claude (Opus, high effort). **Review: Sonnet : high, required and
pending.** Nothing here is accepted until that review runs.

### What landed

| Deliverable | Where | Notes |
| --- | --- | --- |
| Provider/protocol identity | `LlamaModel::for_admitted_editing_v2(profile, session_id)`; `LlamaProfile::digest()` | Identity is `shuttle-llama-admitted-editing-v2:<profile_digest>`, exactly `AdmittedEditSession::provider()`. Planning/fixture keep `shuttle-llama:<digest>`. The adapter is bound to one session ID. |
| Shared turn context | `AdmittedEditTurnContext`, `parse_edit_model_context` (`src/edit_session.rs`) | Replaces the ad hoc `json!` in `edit_model_context`. Encoded through `Value`, so saved bytes are identical to the previous form. The parser requires canonical bytes, the derived session ID, the initial-context hash, and history that agrees with both read budgets. |
| Strict schemas | `v2_tool_schema` (`src/llama.rs`) | Three tools with `additionalProperties:false`, `minItems`/`maxItems`, and integer `minimum`/`maximum`. No `pattern` (see limitations). |
| Request composition | `admitted_editing_v2_wire` | Pure function of profile + session ID + durable context. The prompt projects objective, constraints, allowed files (hash, size, preview) and budgets; saved reads replay as exact assistant/tool pairs with deterministic IDs. Serialized request carries `version:2`, `bounds_revision:1`, protocol, session ID, turn and the offered tools. |
| POST / context check | same | `body ≤ 24,000` and checked `body + max_tokens + 4096 ≤ n_ctx` (carried from Step 0). |
| Transport | `exchange(…, Expect::Json)` | `stream:false`, no `stream_options`. Exactly one `Content-Type`, `application/json` with at most `charset=utf-8`; `Content-Encoding` absent or `identity`. The existing 64 KiB per-artifact cap, GET/POST/GET, no retry/redirect/proxy, and pre/post identity checks are unchanged. |
| Decoder | `src/llama/completion.rs` | Typed envelope, never `Value` (which keeps the last duplicate silently). Known envelope fields reject duplicates; unknown llama.cpp fields are inert. `function` and every argument object deny unknown fields. Arguments are parsed once. Enforces one choice at index 0, assistant role, `tool_calls` finish, one call, nonempty response/call IDs, function type, matching model, no legacy `function_call`, no `error`. Usage is validated and recorded before later rejection. |
| Bounds | decoder + `validate_text_patch_proposal` (`src/workspace.rs`) | Arguments ≤ 24,000 serialized bytes; `ModelReply` ≤ 60,000. Files 1–8, hunks 1–16 per file and ≤ 64 total, old+new ≤ 4,096, paths ≤ 1,024 each and ≤ 2,048 total, canonical path spelling, lowercase BLAKE3, nonempty and non-no-op hunks. The validator is shared: the planner now calls it first, and its own preimage-aware checks stay as a second pass. |
| v1 isolation | `respond_prepared` | Exact saved-vs-recomposed comparison, plus an explicit v2 protocol/version/bounds check before any exchange, with the decoder chosen by mode, never by saved data. |

### Contract refinement recorded in S033

Read tools are offered only while a read can still succeed (not on the fifth
turn, and not when the read count or excerpt-byte budget is exhausted), and the
decoder rejects any tool the saved request did not offer. This is stricter than
"the fifth turn exposes only `record_task_patch`". The S033 clarification
(2026-09-24) also records envelope tolerance and the model-facing projection.

### Carried items closed

- **POST `body + max_tokens + 4096 <= n_ctx`**: implemented with checked
  arithmetic in the v2 composer.
- **Duplicate-key rejection for the internally tagged `TextReadOperation`**:
  the wire never decodes that enum; it uses per-tool `deny_unknown_fields`
  structs. The enum itself (still used for durable observations) was pinned by
  test: duplicate `path`, duplicate `tool` and unknown fields are all rejected.

### Tests

- `src/llama/completion.rs`, 13 unit tests: each tool's decision, single
  escape pass (wire `caf\\n` decodes to a literal backslash and `n`, never LF;
  CRLF and `\u00e9` exact),
  lone surrogate and invalid UTF-8, duplicate keys at envelope/arguments/file/
  hunk level (each asserting `duplicate field`, not merely an error), unknown
  fields/wrong types/coercions, unsafe paths and hashes, unoffered/unknown/
  multiple tools, identity/role/choice/finish/trailing data/SSE body, usage
  optional, validated and retained after rejection, and both sides of every
  serialized and decoded bound, including S033's escape example (4,096 decoded
  control bytes exceeding the 24,000-byte arguments cap).
- `src/edit_session.rs`: `json_object_keys_are_lexically_ordered` pins the
  assumption every `Value`-built identity makes (see findings).
- `tests/llama_edit_v2.rs`, 5 mock-HTTP tests through the real journal session
  (intake → admission → planning with the real profile digest → human grant →
  session):
  - a read turn round-trips, and the next turn replays it byte-exactly;
  - v1 and foreign requests are refused before any network call;
  - transport accepts only identity JSON, caps a chunked body with no
    Content-Length at exactly 65,536 bytes, and refuses a reply after worker
    drift while keeping usage;
  - the fifth turn offers and accepts only `record_task_patch`;
  - the exit gate below.
- Mutation check: disabling the JSON header check and the patch-only tool
  restriction made exactly the two corresponding integration tests fail.

### Exit gate

A synthetic emCP-shaped edit replaces the testing paragraph of an 11,702-byte
`AGENTS.md`. The text is invented to match r4's shape and is not emCP's
content. Measured on the mock:

| Surface | Measured | Bound |
| --- | --- | --- |
| POST body | 4,530 B | 24,000 B |
| Context allowance (`body + 512 + 4096`) | 9,138 | mock `n_ctx` 32,768 |
| HTTP response | 1,066 B | 65,536 B |
| Tool arguments | 556 B | 24,000 B |
| Serialized `ModelReply` | 641 B | 60,000 B |
| Old + new text | 388 B | 4,096 B |
| Tool-call output tokens, strict upper bound | ≤ 573 | default `max_tokens` 512, configurable maximum 2,048 |

The decoded patch also resolves through the pure planner to exactly the
expected postimage, and nothing is written.

**Honest reading of the token row.** The strict bound (every byte-level BPE
token covers at least one byte) proves fit only against the 2,048 maximum, not
the default 512. A typical English ratio of about 3–4 bytes per token puts
the call near 150–190 tokens, well under 512, but that is an estimate. Only
Chunk 7's live usage can settle it. With `thinking` enabled, reasoning tokens
share `max_tokens`. The profile is immutable per run and shared with planning,
so choosing `thinking=false` or a larger `max_tokens` for the qualification
profile is an operator decision to make before Chunk 7.

**Correction to the r4 narrative.** For this file, v1's whole-file arguments
alone come to about 42 KB (every byte as a decimal integer plus comma), which is
*under* 64 KiB. r4 crossed the transport bound only once SSE per-token framing
was added on top. v1 arguments would still exceed v2's 24,000-byte argument
cap, and v2's are about 75× smaller.

### Gate

Native Rust 1.98.0 (`rustc 88d9e12ae 2026-08-18`). `cargo fmt --all --
--check`, `cargo check --locked --tests` and `cargo clippy --locked
--all-targets -- -D warnings` all pass. Focused tests: lib 33 (was 19),
`llama_edit_v2` 5, `llama_adapter` 10, `edit_session` 15, `workspace_text_patch`
12. Adjacent: `task_planning` 10, `verification` 12 (1 ignored). Command log:
`.shuttle/chunk-3-wire-protocol-native-linux.log`.

**Toolchain deviation, recorded per the standing policy.** This session's
device VM had no Rust toolchain and about 400 MB free disk, too little to build
the crate. The gate ran on native Linux Rust 1.98.0 in Claude's isolated cloud
workspace, on a byte-copy of the tracked tree staged from the checkout. It was
not built on the operator's machine. The changed files were then written back
to the checkout.

**Full suite (informational; Chunk 6 owns this gate).** Every binary passes
except three that need the pinned Python 3.14.7 from
`tests/Dockerfile.qualification` (this workspace has 3.11.15):
`evidence_qualification::real_unittest_callbacks_…`, three `live_repair` tests
and `live_repair_cli`, all failing with `test_profile_component_mismatch`. The
**pre-Chunk-3 baseline tree fails identically**, verified in the same
workspace. Per the toolchain policy these fall back to the operator.

### Findings and limitations

- **`serde_json` key order is load-bearing and implicit.** Every identity
  built through `Value` (profile digest, session ID, patch identity, the turn
  context) assumes a lexically ordered map. That holds because no *normal*
  dependency enables `preserve_order`. tree-sitter enables it only for a build
  dependency, which resolver 2+ keeps separate. A future dependency could flip
  it silently. The new unit test fails first if that happens.
- **The approved plan summary is not in the edit context.** The model sees the
  objective and files, not the planning proposal the human approved. For small
  models that is useful scaffolding, but adding it changes the session
  definition, so it is a Chunk 4 candidate and needs an S033 note. Not done here.
- **No `pattern` on `expected_file_hash`.** llama.cpp turns tool schemas into
  grammars. `minItems`/`maxItems`/`minimum`/`maximum` are conservative, but a
  regex pattern's conversion has not been verified against the pinned worker.
  The decoder enforces the hash spelling regardless.
- **Envelope tolerance is deliberate.** Unknown top-level/choice/message fields
  are ignored so a llama.cpp minor version adding metadata does not fail a
  turn. Every field that can carry an executable decision rejects unknowns.
- **Not in this chunk:** orchestration (`respond_prepared` → result → read
  commit or patch preparation), the failure/discard commit for wire-invalid
  replies, and stopping generation of v1 edit requests for new tasks. All are
  Chunk 4. `docs/llama-adapter.md` still describes only the streamed v1
  adapter; Chunk 6 consolidates it.

Git was deliberately left to the operator; nothing was committed by this chunk.

### Next task

- **Blocking first:** Chunk 3 review (Sonnet : high, fresh task). Attack
  surface, in priority order:
  1. Can any byte sequence reach `Decision::AdmittedText*` without passing the
     typed envelope? Look especially for duplicate *unknown-then-known* keys
     and for `Option` fields where `null` and absence are conflated.
  2. Is `admitted_editing_v2_wire` truly a pure function of (profile, session
     ID, context)? Any nondeterminism breaks prepared-request recovery.
  3. Does `parse_edit_model_context` refuse every context `edit_model_context`
     could not have produced, and accept every one it can (including a
     byte-budget-exhausted turn with reads remaining)?
  4. Does offering fewer tools than S033's fifth-turn rule ever strand a
     session that could otherwise still read?
- **Next chunk:** 4, durable preparation and filesystem application (Opus :
  high, fresh-task adversarial Opus : high review).
- **Context pack for 4:** Chunks 1–3 results; S033 including the 2026-09-18,
  -20 and -24 clarifications; `run_admitted_task_edit`; action and request
  transactions; `finish_admitted_text_read` as the read-commit template;
  source capture, path safety, readmission, evidence, offer and acceptance code.
- **Carried into Chunk 4 from the Chunk 3 review:** R1, size-aware read commit
  (see Chunk 4 deliverables).
- **Decide before Chunk 7:** the qualification profile's `thinking` and
  `max_tokens` (see the exit gate's token note).

## Reconciliation result — Windows and Docker Linux gates (2026-09-24)

Performed by Claude (Opus, high effort) in a local Claude Code session on the
operator's Windows 11 machine, against the uncommitted working tree at HEAD
`89d6126` (Chunks 2a, 2b and 3, plus the operator's `.gitignore`, `README.md`
and status edits). It covers the handoff's (`.shuttle/handoff.092426.md`) open
problems 1–4. No source, test or contract file changed: SHA-256 of all 36
`src/`, `tests/llama_edit_v2.rs`, `docs/*.md`, `README.md` and `.gitignore`
files matched before and after the gates. Nothing was committed. The Chunk 3
review and Chunk 4 were not started. The handoff's named prompt,
`.shuttle/reconcile-prompt.092426.md`, does not exist. The work followed the
handoff's open problems and invariants directly.

### Open problem 1 — Windows gate: passed

Toolchain: `rustc 1.98.0 (88d9e12ae 2026-08-18)` on `x86_64-pc-windows-msvc`,
Visual Studio Build Tools 2022 17.14.37710.0 (MSVC 14.44.35207), Python 3.14.7.
Log: `.shuttle/reconcile-windows-gate.log`.

| Step | Result |
| --- | --- |
| `cargo fmt --all -- --check` | pass |
| `cargo check --locked --tests` | pass |
| `cargo clippy --locked --all-targets -- -D warnings` | pass |
| `cargo tree -e normal,features -i serde_json --locked` | no normal dependency enables `preserve_order` |
| `cargo test --locked --no-fail-fast -j 4` | **201 passed, 0 failed, 10 ignored** (standalone helper fixtures) |

Focused binaries on Windows: lib 33, `edit_session` 15, `llama_edit_v2` 5,
`llama_adapter` 10, `workspace_text_patch` 12. The Windows-specific paths named
by the handoff are exercised by passing tests, not merely compiled:
- `file_identity`'s `cfg(windows)` `GetFileInformationByHandle` branch, by
  every `edit_session` read and by the ungated
  `a_hard_linked_target_is_refused_by_the_identity_guard`;
- the canonicalized-root comparison in `fresh_edit_session`, on every passing
  `edit_session` path, where Windows `\\?\` spelling applies;
- the mock-HTTP tests, through `llama_edit_v2` and `llama_adapter`.

Three attempts preceded the pass, and all three are in the log:
1. **`link.exe` not found.** Visual Studio or Build Tools had been removed on
   2026-09-14, between 09:36 and 09:44 (empty `Microsoft Visual Studio\{18,2019,2022,Shared}`
   folders, no `vswhere.exe`, runtimes only in the uninstall list). The last
   Windows-linked test binaries date from 2026-09-13. fmt, check and Clippy do
   not link, which is why they still passed. The operator reinstalled Build
   Tools.
2. **`E0463 can't find crate`** for `cortex_shuttle`, `cortexweave`, `sqlx`
   and others while the rlibs were present on disk, first in `target\`, then
   again in a fresh isolated target directory. That rules out stale state (the
   log's RERUN 2 note guessed stale state; its RERUN 3 note corrects that). A single test target
   (`workspace_text_patch`) built cleanly from the same directory. The failure
   appears only when about 16 test crates compile in parallel, each mapping the
   196 MB `cortex_shuttle` and 148 MB `cortexweave` rlibs, on a host with
   31.4 GB RAM, a 47.4 GB commit limit and about 14 GB free commit at idle,
   while the 6 GiB Docker VM ran at the same time. This was diagnosed as
   memory/commit exhaustion reported by rustc as a missing crate, not a code
   fault.
3. **Pass with `-j 4`.** This caps compile jobs only. Test execution is
   unchanged.

The build ran in an isolated target directory under Claude's scratchpad.
`target\` was left untouched. Windows gates should use `-j 4` (or less
concurrent load) until the host has more commit headroom.

Windows runs 3 fewer tests than Linux, and each is an explicit platform gate.
Linux-only: `linux_cli_sigint_records_cancellation_and_cleans_up`,
`linux_cleanup_includes_descendants_that_create_a_new_session`,
`linux_symlink_and_parent_paths_are_rejected` and
`native_linux_bracketed_paste_preserves_unicode_and_newlines`. Windows-only:
`junction_and_parent_paths_are_rejected`.

### Open problem 2 — pinned-Python tests: passed on both platforms

Docker Linux, using `tests/Dockerfile.qualification` rebuilt unchanged as
`shuttle-qualification:local` (image
`sha256:7a49e23f15df8517ce9de6fd8d869324726dd33208771a87e02092b0102cb86b`):
Rust 1.98.0 with pinned Python 3.14.7 at `/opt/python/bin/python3`. Run shape
matches the historical record: `--init --cpus 2 --memory 6g`, Shuttle source
mounted read-only, separate `shuttle-linux-cargo` (as `CARGO_HOME`) and
`shuttle-linux-target` volumes, no privileged or host-PID mode, no bind-mounted
`target\`. The exact command is on the log's first line. Log:
`.shuttle/reconcile-docker-linux.log`.

| Step | Result |
| --- | --- |
| fmt check, `check --locked --tests`, Clippy `-D warnings` | pass |
| `cargo tree … -i serde_json --target x86_64-unknown-linux-gnu` | no `preserve_order` |
| `cargo test --locked --no-fail-fast` | **204 passed, 0 failed, 10 ignored** |

The five tests that had not passed in any recent gate now pass on both
platforms: `evidence_qualification::real_unittest_callbacks_preserve_failed_and_passing_cases_and_raw_capture`,
`live_repair` (all 4 in the binary, including its 3 pinned-Python tests) and
`live_repair_cli` 1. This confirms that the earlier `test_profile_component_mismatch`
failures were the automation environments' Python 3.11, not code.

The Docker leg and the Windows leg ran concurrently on the same host. The
Docker VM's memory is what made the Windows parallel build run short (see
attempt 2 above).

### Open problem 3 — audit of the 2b "all pass" claim: unsupported as written; corrected

`docs/implementation-status.md` says the 2026-09-20 F1–F3 run passed "`cargo
test --locked`". No log of any 2026-09-20 run is retained in `.shuttle/`. The
only 2b gate log (`chunk-2b-session-ledger-native-linux.log`, 2026-09-18) ran
focused binaries only (`edit_session`, `workspace_text_patch`, lib,
`task_planning`, `verification`). The 2a log ran no full suite either. Every
environment available to those sessions had Python 3.11, in which the five
pinned-Python tests fail. Chunk 3's log shows that the pre-Chunk-3 tree fails
them identically. So the claim was an overclaim when it was written. It is now
true of the current tree, which contains 2b, on both platforms, as of this
reconciliation. A dated correction was added directly after that sentence in
`docs/implementation-status.md`. The original sentence was left intact.

### Open problem 4 — toolchain policy: revised, not rewritten

A dated revision was added under the 2026-09-18 policy in Step 0. The original
paragraphs stand as the record of the device-VM constraint.

### What this changes and what it does not

- Chunks 2a, 2b and 3 now have Windows and Docker Linux full-suite evidence on
  the same tree. This does **not** replace Chunk 3's pending Sonnet : high
  review, and it does not satisfy Chunk 6's gate, which runs on the completed
  diff.
- Open problems 5–7 in the handoff (token budget, plan summary in the edit
  context, `docs/llama-adapter.md`) are untouched.
- Next: the Chunk 3 review (fresh session, Sonnet : high), as listed in
  "Chunk 3 result → Next task".

## Chunk 4 split and results (2026-09-24)

Performed by Claude (Opus, high effort) in a local Claude Code session on the
operator's Windows machine, starting from HEAD `d0bd549` with a clean tree.
**The fresh-task adversarial Opus : high review is required and pending.**
Nothing here is accepted until it runs.

**Chunk 3 review status, reconciled.** This plan's header, "Chunk 3 result"
and `docs/implementation-status.md` all still said the Sonnet : high review was
pending. Commit `d0bd549` contradicts that. Its message cites "the Chunk 3
review's R1". Its decoder tests grew from 13 to 22 and its mock-HTTP tests from
5 to 6, with the new cases matching the review's attack list. And this plan's
Chunk 4 deliverables already carry R1 and the offer/accept parity pin in from
that review. So the review ran and its fixes are committed; only the status
lines were stale. No separate review record exists in `.shuttle/`. This entry
is the correction, and the older lines are left as written.

### Split (Step 0: "moving or splitting a chunk when the current code exposes a safer boundary")

Chunk 4 crosses three durable boundaries and one integration surface, so it
is split the way Chunk 2 was. Each part has its own tests and gate, and the
single adversarial review covers all four.

| Part | Scope | State |
| --- | --- | --- |
| 4a | R1 size-aware read commit; S033 clarification; offer/accept parity pin | Implemented |
| 4b | Atomic patch-reply + prepared-action commit; bounded failure commit for rejected replies | Implemented |
| 4c | Filesystem application: last pre-start boundary, retained-handle writes, read-back, whole-snapshot check, cancel-before-start, unknown-after-start | Implemented |
| 4d | Orchestration loop; CLI and terminal switch from v1 to v2; offer/finalization variant matching; stop generating v1 | Implemented |

Model:effort for all four: Opus : high primary, per the Step 0 table. No
escalation was needed.

### 4a: size-aware read commit (R1)

Recorded as S033's 2026-09-24 Chunk 4a clarification, written before the code
was finalized. Decisions (a) and (b) as the plan required:

- **(a)** When no excerpt fits, the read is refused. The request commits
  `succeeded` with application `refused_read:<reason>`, and no observation is
  stored. The session's `reads_closed_reason` is set (migration 0023, set once,
  never cleared). The session stays open and the run stays ready. Every later
  turn is patch-only.
- **(b)** A shortened excerpt charges exactly the bytes it returned.

Mechanism: `compose_edit_intent` is now the only composer of a turn's intent.
`prepare_admitted_edit_turn` uses it, and the read commit uses it to compose
the exact next turn through the same provider. `finish_admitted_text_read` now
takes that provider and returns `AdmittedReadCommit::{Observed, Refused}`. The
full allowance is tried first, then a verified search for the largest one that
fits.

A design correction found by test: `reads_closed` was first omitted from the
turn context while false, to keep canonical bytes unchanged. A refusal right
after an exactly shortened read then overflowed by the 20-odd bytes of the
added field (`no representable turn follows a refused read`). The field is now
always serialized. `false`→`true` shrinks the encoding, so a refused turn's
continuation is never larger than the refused turn.

Parity pin: `read_offer_matches_journal_acceptance` checks every
(turn, read count, read bytes, reads closed) combination, 307,250 cases.
`AdmittedEditTurnContext::reads_available()` offers a read exactly when
`ensure_read_turn_available`, now four independently messaged clauses, accepts
one.

Tests: `three_maximum_reads_on_ordinary_text_leave_a_patch_turn`,
`a_backslash_dense_file_no_longer_strands_the_session` and
`a_read_that_cannot_fit_is_refused_and_only_a_patch_can_follow`
(`tests/edit_session.rs`), plus
`a_backslash_dense_file_keeps_every_real_request_representable` through the
real adapter (`tests/llama_edit_v2.rs`). Mutation check: with the fit check
disabled (pre-R1 behavior), both journal-level dense tests fail with exactly
the original R1 error, "edit history exceeds request allowance". The ordinary
text test still passes.

### 4b: patch preparation

`Journal::finish_admitted_text_patch` (`src/edit_session/apply.rs`) first
settles the turn with the same guard as reads. It then opens every target
read-only: granted, declared, contained, link-free, single-named, matching its
*admitted* hash, and distinct by native file ID. The pure planner resolves the
patch against those preimages. The projected snapshot must stay within 16 MiB
and the action intent within 60,000 bytes. Freshness is checked again, then
one transaction commits:

- the request result and artifacts;
- the prepared `PatchWorkspaceFiles` action
  (`<run>/admitted-text-patch/<session>`);
- application `admitted_text_patch:<action>`;
- the session's `action_id` and its closure.

A rejected reply instead commits through `commit_edit_turn_failure`, shared
with reads: `reply: null`, a bounded diagnostic with the full error chain,
`discarded:<reason>`, the session closed and the run paused. No file is
written in either path.

### 4c: application

`Journal::apply_admitted_text_patch` runs at most once per action.

- **Succeeded:** it returns the saved result and never writes again.
- **Started, unknown, failed or cancelled:** replay is blocked.
- **Prepared:** it proceeds as follows.
  1. S033 steps 1–4: bindings, including the originating request's application
     marker. Then targets reopened writable. Then an exact reconstruction from
     fresh preimages, which must equal the saved patch.
  2. Step 5: the last pre-start check, covering permission, the full live
     snapshot and every path-to-handle binding.
  3. The started marker commits.
  4. Each target, in canonical order, is written through its retained handle:
     immediately before the write, its path, type, identity and preimage are
     checked again; then seek, write, set length, sync.
  5. Every target is read back.
  6. The post snapshot must *equal* the predicted one: the admitted snapshot
     with only the patched files' sizes and hashes replaced and the Git
     working-tree identity recomputed.
  7. The completion artifact is stored, binding the patch identity, observed
     post hashes and post-snapshot ID. The full post snapshot is stored under
     that ID.

Any check failure before the started marker cancels the action. Nothing was
written, so the result is `cancelled`, inputs are recorded as "unobserved" and
the run is paused. (Corrected by review R4-2: failures of `Journal::start`'s
own run-level guards leave the action prepared and resumable instead.) Any failure after the marker records `unknown` immediately
and pauses the run, rather than waiting for restart recovery. A crash leaves
`started`, which recovery turns into `unknown`. There is no rollback, retry or
manufactured success.

Fault injection uses a doc-hidden stage hook: crash or I/O failure at
`Prepared`, `Started`, `Written(n)` and `Verified`. The hook can also change
files to stand in for a concurrent writer. SQLite triggers make the action
insert and the success commit fail for real. `tests/edit_patch.rs` has 17
tests. Mutation check: each of these guards, removed alone, fails its test:
the pre-write target check, the whole-snapshot comparison, the last pre-start
boundary, exact reconstruction, and immediate unknown after start.

### 4d: orchestration and the v1 cutover

`workspace::run_admitted_task_edit` is now the v2 loop. Its arguments are the
state directory, the workspace, the profile digest, and a factory that builds
the provider for the session it opens.

- It opens or resumes the single session for the current grant. If a patch
  action already exists, it resumes that action once or returns its saved
  success.
- Otherwise it runs at most five turns, each prepared, then started, then one
  POST.
- A read commits its observation or refusal, and the loop continues.
- A patch commits its prepared action, which is then applied.
- Any other outcome (transport, decoder or timeout error, or a decision
  outside the protocol) goes through the new `fail_admitted_edit_turn`.
  Artifacts and usage are retained, the session closes, the run pauses, and
  there is no retry.

`task-edit` and the terminal's edit operation call it through
`LlamaModel::for_admitted_editing_v2`. The v1 orchestrator and whole-file
executor are deleted, so nothing generates v1 edit requests. Offer creation
and acceptance now match either edit variant explicitly, and the `edit_done`
projection bug is fixed. The two task_planning end-to-end tests now run v2:

- `permitted_model_patch_…` covers patch, rerun (which returns the saved
  success with no second inference) and re-admission.
- `review_offer_…` covers patch, re-admission, the admitted suite, the offer,
  acceptance and native finalization.

`tests/llama_edit_v2.rs` adds two orchestrator tests through the real adapter:

- find then patch then apply, exactly two POSTs, with the second replaying the
  find;
- a hostile `run_shell` reply: one POST, a failed and discarded request that
  keeps usage, a paused run, and a rerun that refuses without a POST.

Not covered by 4d, and left to Chunk 6: an old-journal test that takes a
*historical* v1 `WriteWorkspaceFiles` action through offer and acceptance now
that v1 cannot be generated. That path's code only widened a `matches!`.

### Found in passing

- **Fixed:** the new failure commit briefly bound `bounded_json`'s bytes into
  the TEXT `result_json` column. Existing tests never read that row back.
  Caught in review of my own diff before any test depended on it; it now binds
  a string.
- **Pre-existing, fixed in 4d:** `workspace.rs` task view computes `edit_done`
  with `intent_json LIKE '%WriteWorkspaceFiles%'`, but `ToolCall` serializes as
  `"tool":"write_workspace_files"`. The pattern never matches. After a
  successful v1 edit, the task view therefore told the operator "Permission is
  saved. Request the bounded patch" instead of "Re-admit, then run the admitted
  verification suite".
- **Not done, still a candidate:** the approved plan summary is not in the
  edit context (Chunk 3 finding). It changes the session definition and needs
  its own S033 note.
- **Tightened during self-review:** the completion artifact's
  `observed_post_hash` and `observed_post_size_bytes` now come from the bytes
  read back through each handle. They had been computed from the in-memory
  postimage. Both were already verified equal, but a field named "observed"
  should record the observation.
- **Operational limitation, relevant to Chunk 7:** `SourceSnapshot` includes
  the raw `.git/index` hash, and success requires an exact snapshot match. A
  `git status` from an IDE or shell during application refreshes the index. If
  that happens before the started marker, the action is cancelled; after it,
  the action is unknown. This is conservative and is already a documented
  snapshot limitation ("Index stat refreshes may conservatively invalidate
  evidence"). The live emCP run should avoid Git activity in that workspace
  while `task-edit` runs.
- **Left for Chunk 6:** S033's opening line still says "not an implemented
  capability". It is now implemented against mocks but is neither reviewed nor
  live-qualified. Chunk 6 owns rewording it, and `docs/llama-adapter.md`
  still describes only the streamed v1 adapter.
- **Left open:** `open_admitted_edit_session` still checks only the
  definition size, not that turn 0 composes. A turn 0 that cannot be
  represented would close the session at its first preparation rather than
  refuse to open it. No read is involved, so R1's guarantee is unaffected.

### Gate

**Windows (native MSVC), final tree.** Toolchain `rustc 1.98.0 (88d9e12ae
2026-08-18)`. Isolated target directory under Claude's scratchpad, `-j 4`.
Log: `.shuttle/chunk-4-windows-gate.log`.

| Step | Result |
| --- | --- |
| `cargo fmt --all -- --check` | pass |
| `cargo check --locked --tests` | pass |
| `cargo clippy --locked --all-targets -- -D warnings` | pass |
| `cargo tree … -i serde_json` | no `preserve_order` |
| `cargo test --locked --no-fail-fast -j 4` | 228 passed, **1 failed**, 10 ignored |

**The one failure is a pre-existing flake, not Chunk 4.** It was
`llama_adapter::invalid_profiles_and_oversized_contexts_never_dispatch`,
panicking at `llama_adapter.rs:224`. That line is `LlamaProfile::capture`
failing with "connection closed before message completed" against the test's
mock server, before any journal code runs. Evidence:

- The earlier full run
  (`.shuttle/chunk-4-windows-gate-run1-flaky-llama-adapter.log`) failed 6
  different `llama_adapter` tests at the same line.
- A worktree of the untouched baseline `d0bd549` failed 3 of 6 consecutive
  runs of that binary, the same way.
- Three reruns on the final tree, appended to the gate log, all passed 10/10.

A separate task was suggested to fix the mock-server harness without adding
client retries.

Per binary, all other passing: lib 37, acceptance 13, edit_patch 17,
edit_session 18, evidence_qualification 4 (pinned Python 3.14.7 present),
live_repair 4, live_repair_cli 1, llama_edit_v2 9, process_cli 1,
process_execution 19, readmission 5, recovery 8, run_accounting 18,
task_intake 14, task_planning 10, task_workspace 3, terminal_interaction 3,
terminal_layout 11, verification 12, workspace_text_patch 12.

**Docker Linux (pinned image), final tree: pass.** Image
`shuttle-qualification:local`
(`sha256:7a49e23f15df8517ce9de6fd8d869324726dd33208771a87e02092b0102cb86b`,
unchanged since the reconciliation), Rust 1.98.0, Python 3.14.7 at
`/opt/python/bin/python3`, engine 29.7.2. Docker Desktop had stopped
mid-session; Claude started it and the engine answered within 5 s. Run shape
matches the historical record: `--init`, 2 CPUs, 6 GiB, source mounted
read-only, separate `shuttle-linux-cargo` and `shuttle-linux-target` volumes.
The gate script `.shuttle/chunk-4-docker-gate.sh` was mounted read-only at
`/gate.sh`. The exact command is on the log's first line. Log:
`.shuttle/chunk-4-docker-linux.log`.

| Step | Result |
| --- | --- |
| fmt check, `check --locked --tests`, Clippy `-D warnings` | pass |
| `cargo tree … -i serde_json --target x86_64-unknown-linux-gnu` | no `preserve_order` |
| `cargo test --locked --no-fail-fast` | **232 passed, 0 failed, 10 ignored** |

Linux runs 4 more tests than Windows; these are the platform gates listed in
the reconciliation. `llama_adapter` passed 10/10 here.

Git was left to the operator; nothing was committed.

### Next task

- **Blocking:** the Chunk 4 adversarial review (Opus : high, fresh task).
  Nothing in 4a–4d is accepted before it. Attack surface, highest risk first:
  1. Can any path write a byte without a committed started marker? Can any
     path after the marker yield a success that was not fully observed?
     Start from `apply_admitted_text_patch_with_faults`, `write_patch` and
     `cancel_patch`/`patch_unknown`.
  2. Is every S033 step-1–5 check present at the right boundary, and is the
     per-target pre-write check sufficient once earlier targets have changed?
  3. R1: is `compose_edit_intent` really the only composer, so the fit check
     and preparation cannot disagree? Is the refused-read continuation
     argument (clarification item 5) sound for every turn index and every
     adapter?
  4. Does `fail_admitted_edit_turn` or the patch-reply commit ever leave a
     started request that could authorize a second POST?
  5. Is the v1 cutover complete? Can anything still generate a v1 edit
     request, and does anything alias the two edit variants?
- **Then:** Chunk 5 (projections) per the Step 0 table. The task view does
  not yet render v2 hunks, reads, refusals or cancelled/unknown patch actions
  in any detail; that is Chunk 5's deliverable.

## Chunk 4 adversarial review (2026-09-24)

Performed by Claude (Opus, high effort) in a fresh session on the operator's
Windows machine. The review covered the uncommitted Chunk 4 tree on top of
`d0bd549`, read against S033 (including the 2026-09-24 clarifications) and
the attack list above. It did not change any source, test or contract file.
A temporary probe test was run and then removed from `tests/`. A copy is
kept at `.shuttle/chunk-4-review-probe-archive-bit.rs.txt`.

Re-run on this tree (native Windows MSVC, isolated target, `-j 4`): lib,
`edit_patch` 17, `edit_session` 18, `llama_edit_v2` 9 and `task_planning` 10
all pass.

**Verdict: not accepted yet.** No path writes before the started marker, and
no path records a success that was not observed. R4-1 is a liveness defect:
it fails safe, but it would turn a correct Windows edit into `unknown` and
burn the task state. R4-2 is a contract-text overclaim. Fix both, then
re-gate on both platforms.

### Findings

- **R4-1 (required, Medium; Windows liveness; fail-safe). An exact
  post-snapshot prediction ignores the archive attribute.** On Windows,
  `SnapshotFile.permissions` is the full `file_attributes()` mask
  (`verification.rs` `file_permissions`). `expected_post_snapshot` copies
  each patched file's baseline `permissions` unchanged. NTFS sets
  `FILE_ATTRIBUTE_ARCHIVE` (0x20) on an in-place write: probed at
  128 → 32 through a .NET read/write handle. So when any target's archive
  bit is clear at admission, a byte-perfect patch fails `post ==
  ready.expected` and is recorded `unknown`, and the run pauses.
  Reproduced end to end through `apply_admitted_text_patch` by clearing the
  bit with `attrib -a` before intake. Result: `Patch outcome unknown after
  start: post-write declared snapshot differs…`, state `Unknown`, with both
  files holding exactly their postimages. A fresh Git checkout sets the bit,
  but backup and sync tools (`attrib -a`, `robocopy /A-:A`, archive-bit
  backups) clear it. This also contradicts S033's reason for in-place writes,
  "preserving existing file identity/metadata".
  *Fix:* for patched entries only, on Windows, compare `permissions` with
  0x20 masked on both sides, or predict `|= 0x20`. Masking is safer, because
  not every filesystem or SMB server sets the bit. Keep every other
  attribute and every unpatched file exact. The stored post snapshot stays
  the observed one. Add a `cfg(windows)` regression test (the probe is a
  ready starting point) and one sentence to S033's Chunks 4b–4d
  clarification, item 4.
- **R4-2 (required, Low; doc accuracy). "Any failure before the started
  marker cancels" overclaims.** `apply_admitted_text_patch_with_faults`
  cancels failures from `prepare_patch_application` and the last pre-start
  check. It does not cancel failures raised by `Journal::start`'s own guards:
  acceptance seal, execution budget or stall, pending model, run phase. It
  also does not cancel load errors before preparation. Those return with the
  action still `prepared` and the run unchanged. The behavior is safe:
  nothing was written, and after `resume_stall` it can be resumed, which is
  arguably better than burning the task. But S033 clarification item 2, the
  4c text above and `docs/implementation-status.md` all state it absolutely.
  *Fix:* reword to "every check `apply` itself performs before the marker
  cancels; `Journal::start`'s run-level guards leave the action prepared and
  resumable." Changing the code to cancel is the worse option.
- **R4-3 (optional, Low; hardening; pre-existing since Chunk 2).**
  `open_admitted_file` opens the path before checking its type. On Unix, a
  FIFO swapped in at a declared path blocks a read-only open indefinitely.
  That can happen during reads and in `finish_admitted_text_patch`, while
  holding the owner lock and a started request. The writable open happens
  before the marker, and it only succeeds on a FIFO or a device node. That
  needs control of the workspace, which S033 already puts out of scope, so
  this is optional. *Fix:* check `symlink_metadata(..).is_file()` before
  opening, and on Unix open with `O_NONBLOCK` before the post-open
  `is_file` check.
- **R4-4 (optional, Nit).** `apply.rs` `MAX_DECLARED_SOURCE_BYTES` duplicates
  the private `verification::MAX_SOURCE_BYTES`. If the two drift, a
  pre-start rejection becomes a post-start `unknown`. Make the original
  `pub(crate)` and reuse it.
- **R4-5 (info, Chunk 5/7).** CLI `task-edit` after a success fails in
  `main.rs`'s `unusable_reason` pre-check ("write permission is unusable")
  rather than returning the saved success, which only the library entry
  returns. For Chunk 7, the "avoid Git activity" note should also name IDE
  and file-watcher Git refresh: Shuttle's own write can trigger it, and the
  raw `.git/index` hash is compared exactly.

### Attack surface: checked, no defect

1. **No byte before the marker.** Before `start`, targets are only opened:
   read+write, no create, no truncate. Every write is inside `write_patch`,
   which runs only after `start` commits `prepared→started` conditionally.
   The owner lock excludes a second process. A crash at `Started` or at
   `Written(n)`, or an I/O failure there, is recovered as `unknown`. Success
   requires handle read-back, `ensure_bound` after read-back, an exact
   snapshot capture and the fault point to pass. A failed `complete` is
   `unknown`. The generic controller cannot start the v2 action: `drive_step`
   dispatches only after `executor.validate`, and neither fixture nor
   process validation accepts it (input hash or spec mismatch).
2. **S033 steps 1–5.** Present at the boundaries S033 names. Steps 1–4 are in
   `prepare_patch_application`: bindings, originating application marker,
   writable targets with admitted hash and distinct file IDs, and exact
   reconstruction (`plan.patch == *patch`). Step 5 is
   `ensure_edit_bindings_current` plus `ensure_bound` on every handle.
   Per-target pre-write checks (path, type, link count 1, identity,
   preimage) hold once earlier targets have changed. Drift anywhere else is
   caught by the post snapshot, as `unknown`. The only gap is the drift
   window between the step-5 check and the `start` transaction; drift there
   is caught after start and recorded as `unknown`, not `cancelled`. That
   gap is inherent.
3. **R1.** `compose_edit_intent` is the only composer. Both
   `prepare_admitted_edit_turn` and the read-commit fit check call it with
   the same provider (identity checked). Its bounds are the same ones
   `prepare_model_inner` enforces, so they cannot disagree. Across every
   turn index, a refusal commits only after the patch-only continuation
   composes, so item 5 is backed by an explicit check rather than relied
   on. `observe_text` cannot fail at a smaller allowance than one that
   succeeded. Induction: every committed read leaves a representable next
   turn.
4. **No second POST.** `start_model` requires `prepared`. A started or
   unknown request fails `prepare_admitted_edit_turn`'s exact-recomposition
   check and closes the session. Every early return that leaves a request
   `started` (settle guard, a failed commit) becomes `unknown` on the next
   `Journal::open`, and every entry point opens a fresh journal.
5. **v1 cutover.** Nothing in `src/` constructs `for_admitted_editing`. Its
   only caller is the refusal test, as clarification item 6 says. The v2
   session refuses any run with v1 evidence. `apply_model` bails on both
   admitted decision types. Offer and acceptance match the two variants
   explicitly through `is_admitted_workspace_edit`, and `llama.rs` names
   them separately.

### Next task

- Apply R4-1 (code, test, S033 sentence) and R4-2 (wording). Optionally
  apply R4-3 and R4-4. Then run the Windows and Docker Linux gates. R4-1
  touches the success criterion, so a short focused re-review of that diff
  is enough; a full second adversarial pass is not needed.
- Then: Chunk 5, per the Step 0 table.

### Review fixes applied (2026-09-24)

Applied by Claude (Opus, high effort) in the review session, at the
operator's request. R4-3 and R4-4 were not applied. Nothing was committed.

- **R4-1.** `apply.rs` has a new `post_write_view`. On Windows, for the
  patched files only, it clears `FILE_ATTRIBUTE_ARCHIVE` (0x20) and
  `FILE_ATTRIBUTE_NORMAL` (0x80) on both sides of the post-write comparison.
  It then recomputes the declared-working-tree identity from that view.
  NORMAL was not in the review's proposed fix; the first regression run
  found it. Windows reports NORMAL only when no other attribute is set, so a
  file with its archive bit clear reads 128 and becomes 32 once it is
  written, and masking 0x20 alone still failed. On other platforms the view
  is the snapshot unchanged. The stored post snapshot stays the observed
  one.
- **R4-1 tests.** Two `cfg(windows)` tests were added to
  `tests/edit_patch.rs`:
  - `a_patched_file_admitted_with_its_archive_bit_clear_still_succeeds`: the
    archive bit is cleared before intake, the patch succeeds, and the stored
    snapshot records the bit as set;
  - `only_the_patched_files_archive_bit_is_excluded_from_the_comparison`:
    `+h` on a patched file, or `-a` on an unpatched declared file, during the
    write still ends `unknown`.

  The harness gained `Harness::build(clear_archive)`. Mutation check: with
  the mask widened to the whole attribute mask, only the second test fails.
  With the mask removed, only the first test fails.
- **R4-1 contract.** S033's Chunks 4b–4d clarification, item 4, records the
  exclusion as a dated, narrow exception. The clarification's "relax
  nothing" preamble now names it.
- **R4-2.** S033 clarification item 2, the 4c text above,
  `docs/implementation-status.md` and the `apply.rs` comment now say that
  application's own pre-start checks cancel, while `Journal::start`'s
  run-level guards leave the action prepared and resumable. No code
  behavior changed.

**Gates, final tree** (`d0bd549` plus the uncommitted Chunk 4 tree with
R4-1/R4-2):

| Gate | fmt | check | Clippy `-D warnings` | `preserve_order` | `cargo test --locked --no-fail-fast` |
| --- | --- | --- | --- | --- | --- |
| Windows, native MSVC, Rust 1.98.0, `-j 4`, isolated target | pass | pass | pass | none | **231 passed, 0 failed, 10 ignored** |
| Docker Linux, `shuttle-qualification:local` `sha256:7a49e23f…cb86b`, historical run shape | pass | pass | pass | none | **232 passed, 0 failed, 10 ignored** |

Logs: `.shuttle/chunk-4-r4-windows-gate.log` and
`.shuttle/chunk-4-r4-docker-linux.log`. The Docker gate script is
`.shuttle/chunk-4-r4-docker-gate.sh`, and its command is on the log's first
line. The Windows total is the earlier 229 (228 passed plus the one flake)
plus the two new Windows-only tests. Linux is unchanged at 232, because both
new tests are Windows-only. The `llama_adapter` flake did not recur.

**A discarded first Windows run, recorded rather than hidden.**
`.shuttle/chunk-4-r4-windows-gate-run1-stale-mutant-build.log` failed
`a_patched_file_admitted_with_its_archive_bit_clear_still_succeeds`. The
mutation check had restored `apply.rs` with `Copy-Item`, which kept the file
modification time from before the mutant build. Cargo therefore reused the
no-mask mutant's test binary. The binary (16:32:26) was newer than the
source (16:31:19), which confirmed this. The run was stopped, the source
touched, and the gate rerun from scratch; that rerun is the passing log
above. The Docker leg was unaffected: it started after the restore and
compiled in its own target volume. In future mutation checks, restore the
file by rewriting it, not by copying it, or touch it afterwards.

**Next task:** the short focused re-review of the R4-1 diff (`apply.rs`
`post_write_view` and its two tests; Opus : high, or Sonnet : high since the
semantics are now fixed). Once that passes, Chunk 4 is accepted. Then
Chunk 5.

### Optional review fixes applied (2026-09-24)

Applied by Claude (Opus, high effort) at the operator's request, after R4-1
and R4-2. Nothing was committed.

- **R4-3.** `open_admitted_file` (`src/edit_session.rs`) now refuses a
  non-regular target by `symlink_metadata` before it opens anything. The open
  itself moved into `open_without_effect`, which on Linux adds `O_NONBLOCK |
  O_NOCTTY`. If a FIFO or terminal is swapped in after the type check, the
  open still returns at once and never takes a controlling terminal, and the
  existing post-open `is_file` check refuses it. Both flags are inert for a
  regular file, including for the later writes through the retained handle.
  In the ordinary case a FIFO never reaches the opener, because snapshot
  freshness refuses a non-regular declared input first. So the tests pin
  both layers directly:
  - `a_directory_target_is_refused_by_type_before_it_is_opened` runs on
    every platform. On Windows, opening a directory would otherwise fail
    with "Access is denied", so the "regular file" message shows the check
    ran first. Mutation check on Windows: with the pre-open check disabled,
    it fails with exactly that error.
  - Two Linux-only tests: `a_fifo_target_is_refused_before_it_is_opened`,
    read-only and writable, each with a 5 s no-hang bound; and
    `an_open_that_loses_the_race_to_a_fifo_still_returns_at_once`, which
    opens read-only through `open_without_effect`.

  Shuttle supports only Windows and Linux, and `libc` is a Linux-only
  dependency, so the flags are gated on `target_os = "linux"`.
- **R4-4.** `verification::MAX_SOURCE_BYTES` is now `pub(crate)` and
  documented. `apply.rs` uses it, and its duplicate
  `MAX_DECLARED_SOURCE_BYTES` is removed. There is no behavior change.

Mutation check on Linux, in the pinned image: with `O_NONBLOCK` dropped
(`O_NOCTTY` kept), only `an_open_that_loses_the_race_to_a_fifo_still_returns_at_once`
fails, on its 5 s no-hang bound ("a read-only FIFO open must not wait for a
writer: Timeout"). The pre-open FIFO test still passes, as expected, because
the type check refuses the FIFO first. Afterwards the source was restored by
rewriting the file, not copying it, so the mtime moved forward (see the
discarded-run note above).

**Gates, final tree** (`d0bd549` plus the uncommitted Chunk 4 tree with
R4-1..R4-4):

| Gate | fmt | check | Clippy `-D warnings` | `preserve_order` | `cargo test --locked --no-fail-fast` |
| --- | --- | --- | --- | --- | --- |
| Windows, native MSVC, Rust 1.98.0, `-j 4`, isolated target | pass | pass | pass | none | **232 passed, 0 failed, 10 ignored** |
| Docker Linux, `shuttle-qualification:local` `sha256:7a49e23f…cb86b`, historical run shape | pass | pass | pass | none | **235 passed, 0 failed, 10 ignored** |

Logs: `.shuttle/chunk-4-r4b-windows-gate.log` and
`.shuttle/chunk-4-r4b-docker-linux.log`. The Docker gate script is
`.shuttle/chunk-4-r4b-docker-gate.sh`. This Docker leg ran from Git Bash
with direct redirection, so its log streamed live. The R4-1/R4-2 Docker run
had been piped through PowerShell, which buffered its output until the run
exited. Test deltas from the R4-1/R4-2 gates: Windows +1, the directory
test; Linux +3, the directory test and the two FIFO tests.

**Next task (unchanged):** the short focused re-review. It now covers the
R4-1 `post_write_view` with its two tests, R4-3's `open_admitted_file`,
`open_without_effect` and their three tests, and R4-4's constant. Once it
passes, Chunk 4 is accepted. Then Chunk 5.

### Focused re-review of R4-1..R4-4 (2026-09-24)

Performed by Claude (Opus, high effort) in the same session that applied the
fixes, at the operator's request. **That is not independent.** The original
Chunk 4 review was the fresh-task review; this pass re-read the fix diffs
cold, but by the same session that wrote them.

**Verdict: passes after one fix (RR-1).** RR-1 is applied, pinned by a
mutation-checked test, and gated on both platforms. On that basis Chunk 4
can be accepted. Whether a same-session re-review is enough is the
operator's decision.

#### Findings

- **RR-1 (fixed; R4-3 was incomplete).** `AdmittedFile::ensure_bound` still
  re-opened the path with a blocking `File::open`. It runs:
  - at the end of `open_admitted_file`;
  - at the last check before the started marker;
  - in `write_target` immediately before each write;
  - after read-back.

  So a FIFO swapped in after the first open would still hang the process.
  After the started marker, that means a hang while holding the journal
  lock with a started action. It now opens through `open_without_effect`,
  and a swapped-in FIFO fails the identity comparison at once.
  - Test: the Linux-only
    `a_fifo_swapped_in_after_opening_fails_the_binding_check_at_once` opens
    and binds a regular file, replaces the path with a FIFO, and requires
    `ensure_bound` to fail with "identity changed" within 5 s.
  - Mutation check (Linux, pinned image): with the blocking `File::open`
    restored, exactly this test fails ("the binding check must not wait for
    a FIFO writer: Timeout").
- **RR-2 (fixed; nit).** `Harness::build`'s doc comment named a parameter
  `clear`. The parameter is `clear_archive`.

#### Checked, no defect

- **R4-1 logic.** The mask touches only `permissions` of the patched
  entries, and only the ARCHIVE and NORMAL bits. The declared-working-tree
  identity is recomputed from the masked files on both sides, so it equals
  a masked-files comparison, and the Git metadata hashes stay exact. If
  `git` is present on only one side, the snapshots still differ. The stored
  snapshot, `input_after_hash` and completion artifact all use the observed
  snapshot. So later re-admission, evidence and the offer's
  `input_after_hash == evidence snapshot` binding all see the same
  archive-set attributes and stay consistent. The archive bit carries no
  content, so excluding it cannot hide a content change: the hash is still
  compared.
- **R4-1 test coverage.** New mutation: masking the bits on *every*
  declared file fails only
  `only_the_patched_files_archive_bit_is_excluded_from_the_comparison`,
  through its `-a` on `src/other.rs` case. With the earlier two mutations,
  all three ways to get the mask wrong are pinned: too much scope, too many
  bits, no mask. `set_attribute` asserts that `attrib` succeeded, so a
  no-op `attrib` cannot make the success test pass vacuously.
- **R4-3.** The type is checked on the path before opening and on the
  handle after it. `O_NONBLOCK` and `O_NOCTTY` are inert for regular files,
  including the later writes through the retained handle. On Windows a
  reparse target is already refused by `check_absolute_path` before the
  type check.
- **R4-4.** One constant, now shared with `verification.rs`. Nothing else
  referenced the removed duplicate.

#### Out of scope, flagged separately

- `verification::read_bounded`, used by every `SourceSnapshot::capture`,
  has the same pattern: an `is_file` check, then a blocking `File::open`. A
  FIFO that races in there can hang any freshness check. The code predates
  this plan and lives outside the edit session, so it was not fixed here. A
  separate task was suggested to the operator.
- The `llama_adapter` mock-server flake (below) was also suggested as a
  separate task: fix the test harness, with no client retries.

#### Gates, final tree (R4-1..R4-4 plus RR-1/RR-2)

| Gate | fmt | check | Clippy `-D warnings` | `preserve_order` | `cargo test --locked --no-fail-fast` |
| --- | --- | --- | --- | --- | --- |
| Windows, native MSVC, Rust 1.98.0, `-j 4`, isolated target | pass | pass | pass | none | 231 passed, **1 failed (known flake)**, 10 ignored |
| Docker Linux, `shuttle-qualification:local` `sha256:7a49e23f…cb86b`, historical run shape | pass | pass | pass | none | **236 passed, 0 failed, 10 ignored** |

Logs: `.shuttle/chunk-4-r4c-windows-gate.log`, with the reruns appended, and
`.shuttle/chunk-4-r4c-docker-linux.log`. The Docker gate script is
`.shuttle/chunk-4-r4c-docker-gate.sh`. Linux gained one test (the RR-1
test); Windows gained none, because that test is Linux-only.

**The Windows failure is the pre-existing `llama_adapter` flake, not
Chunk 4.** The failing test was
`completed_requests_keep_the_run_profile_fixed_and_missing_usage_unknown`,
at `llama_adapter.rs:224`. That line is `LlamaProfile::capture` in the
test's setup, and the error is "connection closed before message completed"
from the test's own mock server, raised before any journal code runs. The
evidence:
- No `src/llama*` or `tests/llama_adapter.rs` file differs from `d0bd549`.
- The test file references none of the code changed here.
- The untouched baseline failed identically in the Chunk 4 gate.
- Every failure in every run had this same signature, with a different test
  each time.

Reruns of the binary, appended to the log:
- **During the Docker gate** (2-CPU, 6 GiB VM busy): 3 of 3 runs failed,
  with 3, 3 and 1 failures.
- **On the idle host:** 1 of 3 runs failed (10/10, 9/10, 10/10).

That is more frequent than the Chunk 4 gate's three clean reruns, but it
is the same defect. It passes on Linux.

**Next task:** the operator decides on acceptance (see the verdict above).
Then Chunk 5 (projections) per the Step 0 table. The two separately
suggested tasks are independent of Chunk 5.

### Follow-up fixes from the re-review (2026-09-24)

The two separately suggested tasks were done in the same session, at the
operator's request. Nothing was committed.

**1. Snapshot reads are non-blocking (the `verification::read_bounded` FIFO
race).**
- `open_without_effect` moved from `src/edit_session.rs` to
  `src/process.rs` as `pub(crate)`, next to `check_absolute_path`. The edit
  session and verification now share that one helper.
- `read_bounded` keeps its type check on the path, then calls a new
  `read_opened_bounded`. That function opens through the helper and refuses
  a non-regular file on the handle before any read.
- Every `SourceSnapshot::capture` reads through it, so no freshness check
  can block on a FIFO swapped in after the type check.
- Test: the Linux-only
  `verification::fifo_tests::a_snapshot_read_that_loses_the_race_to_a_fifo_fails_at_once`
  checks that the path check refuses a FIFO. It also calls
  `read_opened_bounded` directly, standing in for losing the race, and
  requires a "regular file" error within 5 s.
- Mutation check (Linux, pinned image): with a blocking `fs::File::open`
  restored, exactly this test fails ("a snapshot read must not wait for a
  FIFO writer: Timeout"). The file was restored by rewriting it.
- Gates on the Chunk 4 tree plus this fix. fmt, check, Clippy `-D warnings`
  and `preserve_order` pass on both platforms.
  - Windows: **232 passed, 0 failed**, 10 ignored.
  - Docker Linux: **237 passed, 0 failed**, 10 ignored; +1 for the new
    Linux-only test.

  Logs: `.shuttle/snapshot-fifo-windows-gate.log` and
  `.shuttle/snapshot-fifo-docker-linux.log`; script
  `.shuttle/snapshot-fifo-docker-gate.sh`.

**2. The `llama_adapter` Windows flake: root cause found and fixed in the
test harness.**
- *Cause.* The test's mock HTTP server accepted each connection and then
  waited only **2 s** (`set_read_timeout`) for a complete request. If none
  arrived, it closed the socket without responding or recording the
  request. On this Windows host under memory pressure (a busy Docker VM,
  parallel builds; the reconciliation already recorded commit exhaustion),
  the test process can stall for longer than that between connecting and
  sending. hyper then reports "connection closed before message completed".
- *Evidence.*
  - Every failing binary took about 10 s longer (12.5–13.4 s against about
    2.7 s), which is the stall itself.
  - Failures came in bursts: six tests failed at once on consecutive ports
    in the worst run.
  - In that run, `malformed_incomplete…` failed at `llama_adapter.rs:471`
    with **0 POSTs recorded**, although its client had sent one. The mock
    closes a connection unrecorded only when `read_request` fails.
  - CPU-only load and plain Docker load did not reproduce it in 40+ runs,
    instrumented or not, consistent with a paging stall rather than CPU
    contention.
- *Deterministic proof.* A raw-TCP client that connects and waits 0.5 s
  before sending gets the full response. After waiting 2.5 s it gets 0 bytes
  and `ConnectionAborted`, and nothing is recorded: the gate failures'
  signature.
- *Fix (`tests/llama_adapter.rs` only; the production client is
  unchanged).*
  - The wait is now the named constant `MOCK_REQUEST_READ_TIMEOUT` = 30 s,
    about 3× the observed stall. It is still bounded, so a connection that
    never sends cannot hang teardown.
  - The mock no longer drops a connection silently. It logs "mock server:
    no complete request … after accept; closing the connection unanswered",
    which is captured with a failing test's output.
  - `src/llama.rs` and its one-POST, no-retry, no-redirect transport are
    untouched.
- *Regression test.*
  `the_mock_server_waits_for_a_slow_client_instead_of_dropping_it` connects,
  waits 3 s, and then requires a 200 response with its body and one
  recorded request. Mutation check: with the 2 s wait restored, it fails
  with `ConnectionAborted`, and the new diagnostic line reads "no complete
  request 2.01s after accept".
- *Verification.*
  - `cargo test --locked -j 4 --test llama_adapter`: **20 of 20 consecutive
    runs pass on Windows**, run while the Linux suite looped in Docker. Log:
    `.shuttle/llama-adapter-flake-fix-windows-20x.log`, with fmt, check and
    Clippy `-D warnings` (all exit 0) appended.
  - Docker Linux: 11/11 (`.shuttle/llama-adapter-flake-fix-docker-linux.log`).
- *Git note for the operator.* `tests/llama_adapter.rs` is the only file
  whose index blob is CRLF (`git ls-files --eol`: `i/crlf`), and
  `core.autocrlf=input` is set. `git diff` therefore shows every line
  changed. Its CRLF endings were left as they were. The real change is +42/−1
  under `git diff --ignore-cr-at-eol`. Committing it under the current
  config will store the file as LF.

## Chunk 4 acceptance (2026-09-24)

The operator accepted Chunk 4 (4a-4d, with review fixes R4-1..R4-4, RR-1, RR-2
and the follow-up fixes) on 2026-09-24, on the evidence recorded above: the
fresh-task adversarial review, the same-session focused re-review, and passing
gates on both platforms. The acceptance decision is the operator's and was
given in chat. The focused re-review was not independent, and the operator
accepted it on that basis. This does not satisfy Chunk 6's independent audit or
Chunk 7's live qualification.

## Chunk 5 result: operator review projections (2026-09-24)

Performed by Claude (Sonnet, medium effort) in a local Claude Code session on
the operator's Windows machine, starting from HEAD `ae0d427` with a clean tree.
**Review: Sonnet : high, required and pending.** Nothing was committed.

### What landed

| Deliverable | Where |
| --- | --- |
| Pure, bounded review model and renderer | `src/edit_review.rs`: `EditReview`, `EditSessionReview`, `TaskEditReview`, `escape_display` |
| Session ledger summary (turns, reads, refusals, rejected turns, closure) | `load_session_review`; tolerates journals without migration 0022 or 0023 |
| Hash-verified completion artifact, read-back per file | `load_change_review`; a missing or mismatched artifact is stated, never shown as a match |
| Task view lines and action labels | `workspace.rs` `TaskReader::view`; unknown/started actions add an inspect line to `pending` |
| Structured review | `TaskReader::edit_review`; `TaskAcceptanceOfferView.change` |
| CLI | `task-edit-review [--json]`; offer respond prints the edit before the confirmation; Ctrl+R now prints the task offer review |
| Docs | S033 clarification (2026-09-24, Chunk 5), implementation status, `docs/llama-adapter.md`, `docs/terminal-qualification.md` |

No stored byte, migration, bound or authority changed. The only production
change outside the projection is `PATCH_PREPARED` becoming `pub(crate)`.

### Limits and safety

- Rendered hunks show at most 1,024 bytes per side and 160 lines, cut on a
  character boundary, each cut with an explicit "not shown" count. The JSON
  review keeps every hunk.
- Saved text is escaped: backslash, tab and CR are spelled out, and every other
  control, bidirectional or invisible format character is shown as a `\u{..}`
  escape. CRLF is therefore visible in a hunk.
- Historical whole-file actions show paths, hashes and sizes, never bytes.
- The review reads saved state only, so a later workspace edit cannot change it
  (asserted by a test).

### Two pre-existing defects found and fixed

- `TaskOfferRespond` told the operator to review "the exact task edit above",
  but the offer view held only IDs and a hash.
- Ctrl+R on a general-task offer called the generic `acceptance_review`, which
  cannot find a task offer. It now prints the task offer review.

### Tests

- `src/edit_review.rs`, 9 unit tests: escaping, exact hunks with read-back,
  differing and missing read-back, every blocked state, deletion and long-hunk
  bounds (with a multibyte cut), the line cap, historical whole-file actions,
  non-edit actions, and a pre-0022 and 0022-only journal.
- `tests/edit_patch.rs`, 5 new tests through the real journal: no edit, prepared
  then completed (with the workspace changed afterwards), rejected, cancelled,
  unknown. Each also requires the task view to carry the same lines.
- `tests/llama_edit_v2.rs`: the find-then-patch review through the real adapter,
  and the rejected hostile reply. `tests/edit_session.rs`: a refused read.
  `tests/task_planning.rs`: the offer's `change`.
- `tests/terminal_layout.rs`: the review renders at 42x18, 60x24, 80x24 and
  120x36 for a completed and an unknown edit, with hostile text never reaching
  the buffer raw.
- Mutation check: dropping the review lines from the task view fails 4 of the 5
  new `edit_patch` tests.

### Gate

| Gate | fmt | check | Clippy `-D warnings` | `preserve_order` | `cargo test --locked --no-fail-fast` |
| --- | --- | --- | --- | --- | --- |
| Windows, native MSVC, Rust 1.98.0, `-j 4` | pass | pass | pass | not rerun (no dependency change) | **249 passed, 0 failed, 10 ignored** |
| Docker Linux, `shuttle-qualification:local` `sha256:7a49e23f…cb86b`, historical run shape | pass | pass | pass | none | **254 passed, 0 failed, 10 ignored** |

Logs: `.shuttle/chunk-5-windows-gate.log`, `.shuttle/chunk-5-docker-linux.log`;
script `.shuttle/chunk-5-docker-gate.sh`. The Windows leg used the repository's
`target\` directory, and ran after the Docker leg finished, not alongside it.
The `llama_adapter` flake did not recur.

### Not done, still open

- The plan-summary-in-edit-context candidate and `open_admitted_edit_session`'s
  turn-0 check (Chunk 4 notes) are untouched.
- `docs/llama-adapter.md` still describes the streamed v1 adapter at length;
  Chunk 6 consolidates it. S033's opening "not an implemented capability" line
  is likewise Chunk 6's.
- Chunk 4 is accepted (see "Chunk 4 acceptance").

### Next task

- **Blocking:** Chunk 5 review (Sonnet : high, fresh task). Attack surface:
  (1) can any saved string reach a terminal row unescaped, including `pending`,
  `reason` and turn notes? (2) can a review ever say "matches" without an
  observed hash? (3) do the older-journal paths read without migrating?
- **Then:** Chunk 6 (Sonnet : medium primary, Opus : high independent audit).

## Chunk 5 review (2026-09-24)

Performed by Claude (Sonnet, high effort) in a fresh local session, against the
uncommitted Chunk 5 tree on top of `ae0d427`. I read `src/edit_review.rs` in
full, the `workspace.rs`, `main.rs`, `edit_session.rs` and `apply.rs` diffs, and
the Chunk 5 doc changes. I ran three temporary probe tests and deleted them.
No source, test or contract file changed. I did **not** rerun the full Windows
or Docker gates, and I only skimmed the new tests in `tests/edit_patch.rs`,
`tests/terminal_layout.rs` and `tests/llama_edit_v2.rs`. The gate results above
are the implementer's, not re-observed.

**Verdict: not accepted yet.** No path shows "matches" without an observed
hash. R5-1 is a real gap against a claim the docs make. Fix R5-1, and reword or
fix R5-2 and R5-3. The rest are optional.

### Findings

- **R5-1 (required, Medium; the escaping claim is stronger than the code).**
  `escape_display` escapes `char::is_control` (C0, DEL and C1, so U+009B is
  covered) and a short list of format ranges. It leaves other invisible or
  bidirectional characters raw. Probed as passing through unescaped: U+061C
  (Arabic letter mark, a bidi control), U+E0001 and U+E0041 (tag characters,
  invisible), U+00AD, U+034F, U+FE0F, U+180E, U+2028, U+2029, U+206A and
  U+FFF9. S033 clarification item 4 says "every other control, bidirectional or
  invisible format character is shown as `\u{..}`", and `docs/llama-adapter.md`
  says bidirectional overrides "cannot reach the terminal". The hunk text is
  model output, and hidden or reordered text in a hunk defeats the review's
  purpose. The `--json` paths are weaker still. `serde_json` escapes only
  U+0000–U+001F, so `task-edit-review --json` and the offer review's JSON dump
  print DEL, C1 controls (U+009B), U+202E and tag characters raw. I probed that
  too. `main.rs`'s comment that "JSON escapes … terminal control characters" is
  therefore true only for C0.
  *Fix:* widen the set (all `Cf`, `Zl`/`Zp`, variation selectors, tags,
  U+061C and U+00AD are the classes to cover; no new dependency is needed,
  hard-code the ranges) and add one test per class. For the two JSON prints,
  either escape the same set in the string values or state in the docs that
  JSON output is not terminal-safe beyond C0. Otherwise reword the docs to
  name exactly what is escaped.
- **R5-2 (Low; silent truncation).** `bounded_reason` takes 240 characters of
  the *escaped* text but appends `…` only if the *raw* text exceeds 240. A
  100-character all-control reason (600 characters escaped) is cut mid-escape
  with no marker. Probed. This contradicts "each cut marked". Compare the
  escaped length, and cut on an escape boundary.
- **R5-3 (Low; the read-only claim is wider than the code).** S033 item 1 says
  the review never "recovers a journal" and uses "the same read-only connection
  as the task view". That holds for `TaskReader::view` and
  `task-edit-review`: `TaskReader::open` is `read_only(true)`, and
  `load_session_review` tolerates a missing table or column. It does not hold
  for the offer's new `change` field or for Ctrl+R. Those go through
  `task_acceptance_offer_view`, which calls `Journal::open`, so they migrate and
  recover, and the offer view also writes `stale_reason`. That is pre-existing,
  and Ctrl+R's old route also used `Journal::open`, so nothing regressed. But
  the clarification should be scoped to the two read-only surfaces. Also, the
  older-journal unit test uses hand-built tables. No test opens a real journal
  at migration 0021 or 0022 through `TaskReader`.
- **R5-4 (Low; silent absence).** For a succeeded v2 action whose completion
  artifact is present but has no entry or no `observed_post_hash` for a file
  (or is not JSON), the review prints nothing: no read-back line and no
  limitation. Only a *missing or hash-mismatched* artifact is stated. The
  "matches" answer stays safe, but "absent is not matched" is only half visible.
  Add a "read-back not recorded" line.
- **R5-5 (Nit).** In `load_session_review`, the `a.starts_with("discarded: ")`
  arm is unreachable, because the `"discarded:"` arm precedes it. Its only
  producer is `discard_model`, which serves planning and fixture requests that
  the query never joins. Delete the arm.
- **R5-6 (withdrawn; my error).** I wrote that `escape_display` passes newlines
  through, so a saved path or action ID holding `\n` could break the
  one-row-per-entry rule. That is wrong: `char::is_control` includes `\n`, so
  it was already escaped. Only the doc comment ("Newlines are the caller's line
  structure") was misleading. It is corrected, and a test now pins the
  behavior.

### Attack surface: checked

1. **Saved strings reaching a terminal row.** `pending` gains only Shuttle IDs
   and one fixed sentence. Session `outcome`, `reads_closed_reason` and every
   turn note pass through `bounded_reason`. Paths, action IDs and hunk text are
   escaped. The remaining leak is the escape set (R5-1), not a missing call
   site. I did not verify how ratatui treats unescaped control graphemes, so
   nothing here relies on the TUI filtering them.
2. **"Matches" without an observed hash: no.** `observed_matches` is `Some`
   only when the artifact records a hash, and the artifact is fetched by
   `artifact_hash` and rejected if `blake3(bytes)` differs. A missing artifact
   adds a limitation and shows no read-back line.
3. **Older journals.** `TaskReader` is read-only and does not migrate. The
   session query checks the table and the `reads_closed_reason` column first
   (see R5-3 for the offer path and the test gap).

### Review fixes applied (2026-09-24)

Applied by Claude (Sonnet, high effort) in the review session, at the
operator's request. That is the same session that found them, so any
re-check is not independent. Nothing was committed. R5-1 through R5-5 are
all applied.

- **R5-1.** `edit_review.rs` has a new `is_invisible_or_directional` list,
  used with `char::is_control` by `escape_display`: U+00AD, U+034F, U+061C,
  U+115F/1160, U+17B4/17B5, U+180B–180F, U+200B–200F, U+2028–202E,
  U+2060–206F, U+3164, U+FE00–FE0F, U+FEFF, U+FFA0, U+FFF0–FFFB,
  U+1BCA0–1BCA3, U+1D173–1D17A and U+E0000–E0FFF. It is a hand-written list,
  not all of `Cf`, and S033 item 4 now says so. A new `json_terminal_safe`
  rewrites everything `serde_json` leaves raw (DEL, C1, the list above) as
  `\uXXXX`, with surrogate pairs above U+FFFF. It keeps the pretty-printer's
  layout newline. Its first version escaped that newline too, which broke the
  JSON; the round-trip test caught it. It is applied to `task-edit-review
  --json`, the offer review print, and `print_acceptance_review` (whose
  comment overclaimed). Other CLI JSON prints, such as `task-write-status`,
  are not passed through it.
- **R5-2.** `bounded_reason` now escapes per character and cuts between
  escapes at 240 displayed characters, always adding `…`.
- **R5-3.** S033 item 1 is rewritten to scope the read-only, no-migrate claim
  to `TaskReader` surfaces and to say the offer view uses `Journal::open`.
  New `tests/edit_review_journal.rs` builds a real journal, then strips it to
  the pre-0023 and pre-0022 shapes with real DDL and reads it through
  `TaskReader`. It also asserts the read did not migrate it back. It has no
  session rows, so it proves the queries prepare and return `None` against
  the real schema, not the row-level rendering.
- **R5-4.** A succeeded v2 patch with an artifact but no read-back hash for a
  file now shows "No read-back hash is recorded for: <paths>."
- **R5-5.** The unreachable `"discarded: "` arm is deleted.
- **Tests.** Unit: `every_invisible_or_directional_class_is_escaped_not_just_controls`,
  `json_output_is_terminal_safe_and_still_the_same_value`,
  `a_long_reason_is_cut_between_escapes_and_always_marked`,
  `a_succeeded_patch_says_when_no_read_back_hash_was_recorded`. Integration:
  `real_journals_older_than_the_edit_session_migrations_still_review`.
  Mutation check: with the list disabled (controls only), the two escape tests
  and the JSON test fail. The JSON test first passed under that mutation
  because it reused the same predicate. It now asserts against a literal
  character list.
- **Docs.** S033 Chunk 5 items 1, 2 and 4, `docs/llama-adapter.md` and
  `docs/implementation-status.md`. `docs/llama-adapter.md` has mixed line
  endings in the index. An edit briefly normalized it to CRLF, and each line's
  original ending was restored, so its diff is only the intended lines.

### Gates, final tree

| Gate | fmt | check | Clippy `-D warnings` | `preserve_order` | `cargo test --locked --no-fail-fast` |
| --- | --- | --- | --- | --- | --- |
| Windows, native MSVC, Rust 1.98.0, `-j 4`, repository `target\` | pass | pass | pass | not rerun (no dependency change) | **254 passed, 0 failed, 10 ignored** |
| Docker Linux, `shuttle-qualification:local` `sha256:7a49e23f…cb86b`, historical run shape | pass | pass | pass | none | **259 passed, 0 failed, 10 ignored** |

Both are +5 over the Chunk 5 gates: four new unit tests and the new
integration test. The `llama_adapter` flake did not recur. The legs ran one
after the other. Logs: `.shuttle/chunk-5-fixes-windows-gate.log`,
`.shuttle/chunk-5-fixes-docker-linux.log`; script
`.shuttle/chunk-5-fixes-docker-gate.sh`.

## Chunk 5 acceptance (2026-09-25)

The operator accepted Chunk 5 (operator review projections, with review fixes
R5-1..R5-5) on 2026-09-25, on the evidence recorded above: the fresh-session
Sonnet : high review, the fixes and their tests, and passing gates on both
platforms (Windows 254, Docker Linux 259, none failed). The acceptance
decision is the operator's and was given in chat. The fixes were applied in
the review session and had no independent re-check, and the operator accepted
it on that basis. Two limits carry forward: the escape list is hand-written,
not all of Unicode `Cf`, and other CLI JSON prints such as `task-write-status`
do not pass through `json_terminal_safe`. This does not satisfy Chunk 6's
independent audit or Chunk 7's live qualification.

### Next task

- Chunk 6 (consolidation), per the Step 0 table: Sonnet : medium primary,
  Opus : high independent audit. Its open items include S033's opening "not an
  implemented capability" line and `docs/llama-adapter.md`'s v1 description.

## Chunk 6 result: consolidation (2026-09-25)

Performed by Claude (Sonnet, medium effort) in a local Windows session,
starting from HEAD `c6b4302` with a clean tree. **Independent audit: Opus :
high, required and pending.** Nothing was committed.

### What changed

| Deliverable | Where |
| --- | --- |
| S033 opening reworded (implemented against mocks, not live-qualified) | `docs/decisions.md` |
| v2 patch protocol documented; streamed section scoped; stale "not an edit protocol" line fixed | `docs/llama-adapter.md` |
| Historical v1 offer, accept and finalize test, with a no-bytes review assertion | `tests/task_planning.rs` |
| Saved v1 edit action blocks a v2 session before any POST | `tests/task_planning.rs` |
| Linked parent directory never yields outside bytes (symlink or junction) | `tests/edit_session.rs` |
| Exhaustive path-spelling table and an ordinary-names table | `src/workspace.rs` `path_tests` |
| Reserved-name check: trailing space or dot before the extension, superscript digits | `src/workspace.rs` |
| Status entry, S033 clarification | `docs/implementation-status.md`, `docs/decisions.md` |

The README and `docs/terminal-qualification.md` were checked and needed no
change: they already describe the v2 review surfaces and do not claim more.

### Limits the auditor should know

- The junction/symlink test is refused by the run's snapshot check, not by
  the per-component link check. That check is untested in isolation.
- The path table gap was found by writing the table. The `CON .txt` claim
  about Windows behavior was not observed on a real Windows file system. The
  refusal is defensive.
- `CONIN$` and `CONOUT$` are not in the reserved list. The `:` rule does not
  cover `$`. Targets must also be existing regular files, so a device fails
  the type check.
- Chunk 5's two carried limits still hold: a hand-written escape list and
  unfiltered other CLI JSON prints.
- Still open: the plan-summary-in-edit-context candidate, `open_admitted_edit_session`
  not checking that turn 0 composes, and the qualification profile's
  `thinking` and `max_tokens` decision before Chunk 7.

### Gate

| Gate | fmt | check | Clippy `-D warnings` | `preserve_order` | `cargo test --locked --no-fail-fast` |
| --- | --- | --- | --- | --- | --- |
| Windows, native MSVC, Rust 1.98.0, `-j 4` | pass | pass | pass | none | **259 passed, 0 failed, 10 ignored** |
| Docker Linux, `shuttle-qualification:local`, historical run shape | pass | pass | pass | none | **264 passed, 0 failed, 10 ignored** |

Both are +5 over the Chunk 5 fixes gate: three integration tests and two unit
tests. Logs: `.shuttle/chunk-6-windows-gate.log` (test result lines only),
`.shuttle/chunk-6-docker-linux.log`; script `.shuttle/chunk-6-docker-gate.sh`.
A first Docker attempt failed at launch because Git Bash rewrote the `/src`
path. It was rerun with path conversion off. An earlier Windows run was
stopped on purpose, because the validator changed after it started.

### Next task

- **Blocking:** the Chunk 6 independent audit (Opus : high, fresh task). Its
  exit gate is no untested safety claim, silent limit increase, compatibility
  ambiguity, or documentation claim stronger than observed behavior.
- **Then:** Chunk 7, the live emCP qualification. The operator decides the
  qualification profile's `thinking` and `max_tokens` first.

## Chunk 6 independent audit (2026-09-25)

Performed by Claude (Opus, high effort) in a fresh local session on the
operator's Windows 11 machine (build 22631). It covered the uncommitted
Chunk 6 tree on top of `c6b4302`, read against S033, the Chunk 6 result
above, and the exit gate: no untested safety claim, silent limit increase,
compatibility ambiguity, or documentation claim stronger than observed
behavior. No source, test or contract file was changed. Mutations and
probes were applied temporarily. Each file was then restored by rewriting and
touched, and `git diff --stat` again showed exactly the Chunk 6 diff
(6 files, +423/−13). Probe copies:
`.shuttle/chunk-6-audit-probe-linked-parent.rs.txt` and
`.shuttle/chunk-6-audit-probe-historical-v1.rs.txt`.

**Verdict: not passed yet.** No safety defect was found. Every behavior the
docs describe held under probe. But one safety claim has no test (A1), and
two statements are stronger than, or contrary to, observed behavior (A2,
A3). A test's documented provenance is also false (A4). All four are small.
Fix them, then run a focused re-check. A full second audit is not needed.

### Gates, re-observed on the unmodified Chunk 6 tree

| Gate | fmt | check | Clippy `-D warnings` | `preserve_order` | `cargo test --locked --no-fail-fast` |
| --- | --- | --- | --- | --- | --- |
| Windows, native MSVC, Rust 1.98.0, `-j 4`, repository `target\` | pass | pass | pass | none | **259 passed, 0 failed, 10 ignored** |
| Docker Linux, `shuttle-qualification:local` `sha256:7a49e23f…cb86b`, historical run shape | pass | pass | pass | none | **264 passed, 0 failed, 10 ignored** |

Both match the Chunk 6 record. The legs ran one after the other, Docker
first. Logs: `.shuttle/chunk-6-audit-windows-gate.log` (full output) and
`.shuttle/chunk-6-audit-docker-linux.log`, whose first line is the exact
`docker run` command and whose second is the image ID. Script:
`.shuttle/chunk-6-audit-docker-gate.sh`.

### Findings

- **A1 (required, Medium; untested safety claim). The v1 *request* half of
  the v1-evidence refusal has no test.** `open_admitted_edit_session`
  (`src/edit_session.rs` ~L667–680) refuses a run that holds a saved
  `admitted_task_edit_v1` request, a v1 serialized request, or an
  `AdmittedPatch` reply. The new `docs/llama-adapter.md` text says so ("any
  saved v1 whole-file edit request or action"), and so does S033.
  - Mutation M2: with that `ensure!` deleted, all 120 tests in lib,
    `edit_session`, `llama_edit_v2`, `task_planning`, `edit_patch` and
    `edit_review_journal` still pass.
  - `legacy_and_foreign_requests_never_reach_the_v2_decoder` covers
    `respond_prepared` and turn preparation, not session opening.
  - The behavior itself is correct. A probe built an r4-shaped journal (a
    failed v1 request, no action) and got "legacy v1 edit evidence requires a
    fresh task state" with 0 POSTs.
  - *Fix:* add that probe as a test and assert the exact message.
    `prepare_model` is `pub(crate)`, so the probe inserts the row with raw
    SQL through the real state transitions. A unit test inside `src/` is the
    alternative.
- **A2 (required, Medium; documentation stronger than behavior). The new
  application paragraph in `docs/llama-adapter.md` reintroduces the R4-2
  overclaim and drops R4-1.**
  - "A failure before the started marker cancels the action" is the exact
    wording that R4-2 corrected in S033 item 2. `Journal::start`'s run-level
    guards (acceptance seal, execution budget or stall, pending model, run
    phase) leave the action `prepared` and resumable.
  - "records success only if the whole snapshot equals the predicted
    post-edit snapshot" omits S033 item 4's dated exception. On Windows, the
    ARCHIVE and NORMAL attributes of the patched files are excluded from that
    comparison.
  - *Fix:* reuse S033 items 2 and 4's wording, or link to them.
- **A3 (required, Low; the preserved r4 record is misdescribed).**
  `task-edit-review` on a copy of r4 prints "No admitted edit has been
  attempted for this task." r4's v1 edit request reached the worker and
  failed at the transport bound. `load_task_edit_review` looks only at v2
  sessions and edit actions, so any v1-request-only journal gets this false
  sentence. With `--json` it prints `null`. Outcome 1 says r4 is "never
  reinterpreted under the new protocol". The task view is not affected (see
  below).
  - *Fix:* when the journal holds an `admitted_task_edit_v1:` request, say
    that a historical v1 whole-file edit request is recorded and that
    `task-view`/`run-status` describe it, with no bytes. At minimum, change
    the sentence to "No v2 edit session or edit action is recorded."
- **A4 (required, Low; false test provenance).** The helper
  `record_v1_whole_file_action` in `tests/task_planning.rs` says "The bytes
  match what the removed executor stored." They do not. The removed
  executor (`d0bd549` `run_admitted_task_edit`) stored a different shape:
  - action ID `<run>/admitted-write/<permission>`, where the test uses
    `historical-v1-edit`;
  - `Grant { revision: 2 }`, where the test uses 1;
  - `input_hash` = the permission's *pre*-edit snapshot, where the test uses
    the post-edit snapshot;
  - artifact `{permission_id, paths, post_snapshot}`, where the test uses
    `{"historical":true}`;
  - a granted permission and an `admitted_task_edit_v1` request whose
    `AdmittedPatch` reply carries the bytes, with application
    `admitted_task_edit:<action>`, where the test has neither.

  A probe built that exact shape. It offered, accepted and finalized. The
  task view, `task-edit-review` and the offer's `change` all showed the path,
  sizes and hashes, and none showed the replacement bytes. So the
  compatibility claim holds, but only the probe observed it.
  - *Fix:* at minimum, correct the comment. Better, replace the fixture with
    the faithful shape from the archived probe.
- **A5 (Low; a code comment states unobserved Windows behavior).**
  `reserved_windows_component` says "`CON .txt` names the console device."
  This was probed with `GetFullPathNameW` and real file creation on this
  host (Windows 11 build 22631):
  - Absolute paths, which are what Shuttle joins: only bare `NUL` resolves
    to a device. `CON`, `CON.txt`, `CON .txt`, `COM¹`, `CONIN$` and
    `CONOUT$` all create ordinary files.
  - Bare relative names: `CON`, `COM¹`, `CONIN$` and `CONOUT$` are devices,
    and `CON .txt` is not.

  So the superscript rule is observed, and the `CON .txt` rule is defensive.
  Python's `os.path.isreserved` also treats `CON .txt` as reserved. The
  refusal only tightens and stays. *Fix:* reword the comment to say it is
  defensive. Also consider adding `CONIN$` and `CONOUT$`, which are devices
  here in the same sense as `COM¹`. S033 rule 2 says "reserved device
  components", and adding them is tightening only. Separately,
  `docs/implementation-status.md` calls the table "exhaustive"; it is not.
- **A6 (Low; test proves less than its name; the limitation is misstated).**
  `a_linked_parent_directory_never_yields_outside_bytes` gives the outside
  file *different* bytes, so a plain hash mismatch could refuse it. In fact
  the refusal is "Declared inputs cannot be revalidated: symlink path is not
  allowed". That is the per-component `check_absolute_path`, run inside
  `SourceSnapshot::capture`'s `inventory`. Rust reports a junction as a
  symlink.
  - Probes: a **byte-identical** junctioned `src` and a junctioned workspace
    root are refused with the same message.
  - So "refused by the run's snapshot check, not by the per-component link
    check" is imprecise. The per-component check does fire; what is untested
    in isolation is the edit path's own copy.
  - Mutation M4, removing `check_absolute_path` and the containment check
    from `open_admitted_file` and `ensure_bound`: nothing fails, as disclosed.
  - That copy is redundant. Writes go through handles opened by
    `open_admitted_file`, and `ensure_bound`'s file-ID comparison stays. The
    step-5 snapshot capture re-runs the per-component check, and every read
    must match the admitted hash. So a swapped-in link can neither yield
    different bytes nor redirect a write.
  - *Fix (optional):* use identical bytes and assert the message, so the test
    proves a link refusal. Reword the limitation.
- **A7 (Nit).** `a_saved_v1_edit_action_blocks_a_v2_session_before_any_inference`
  asserts only "fresh task state", which three different refusals share.
  Assert "legacy whole-file action". Mutation M1, deleting that `ensure!`,
  is still killed, because the edit then runs.
- **A8 (Nit).** S033's new opening says Chunks 1–5 were reviewed "2026-09-18
  to 2026-09-25". Chunk 1 was implemented and Codex-reviewed on 2026-09-16,
  and 2a had no reviewer by design. Its "Chunk 6's independent audit is
  pending" clause will need updating on acceptance.

### Checked, no defect

- **No silent limit increase.** The diff changes no constant, bound, grant,
  migration or stored byte. The only production change is the reserved-name
  refusal, which only tightens. `ordinary_names_pass_through_unchanged` pins
  that look-alikes such as `com10.rs`, `Console.rs` and `nullable.rs` stay
  valid.
- **Chunk 6's mutation claim reproduced (M3).** With the stem trim and the
  superscript mapping reverted, `unsafe_spellings_are_never_canonical_grant_keys`
  fails at `a/CON .txt`.
- **r4 is intact and still reads the same.** All three files match
  `.shuttle/chunk-0-r4-source-hashes.json`. On a scratch copy, the current
  binary's `task-view --json` is equivalent to the frozen
  `docs/manual-qualifications/emcp-2026-09-16-r4/task-view.json`, so only
  `task-edit-review` is affected (A3). *Info:* opening the copy read-only
  created `journal.sqlite-shm`/`-wal` sidecars, and the main file's hash was
  unchanged. Projections of retained evidence should keep targeting copies,
  as the r4 record already does.
- **Every number in the new `docs/llama-adapter.md` section matches the
  code.** Five turns; four reads of at most 2,048 bytes each and 6,144 in
  total; 128 lines; eight matches, with the ninth flagged as truncated;
  8 files; 16 hunks per file and 64 in total; a 4,096-byte aggregate of old
  plus new text; `stream:false`; the 64 KiB cap.
- **`README.md` and `docs/terminal-qualification.md`** make no stale v1 claim,
  and their review descriptions match `edit_review.rs`.
- **Restart coverage** exists at every durable boundary: `edit_patch` fault
  stages, `edit_session` interrupted turns, adapter recomposition in
  `llama_edit_v2`, and orchestrator reruns in `task_planning`.

Not re-verified: the pre-existing G2 statement that "continuing needs a
fresh task state with a new admission and grant", as opposed to a re-grant
in the same state directory.

### Next task

- Fix A1–A4 (tests, wording and one display sentence). Optionally fix A5–A8.
  Re-gate both platforms. Then run a focused re-check of those diffs
  (Sonnet : high or Opus : high). It should confirm A1's test kills M2, and
  that `docs/llama-adapter.md` now agrees with S033 items 2 and 4.
- Chunk 7 stays blocked on that re-check and on the operator's
  `thinking`/`max_tokens` decision.

### Audit fixes applied (2026-09-25)

Applied by Claude (Opus, high effort) in the audit session, at the
operator's request. All eight findings are applied. Because the session that
found them also fixed them, the focused re-check is still owed and must not be
this session. Nothing was committed.

- **A1.** New `a_saved_v1_edit_request_alone_blocks_a_v2_session_before_any_inference`
  (`tests/task_planning.rs`). It builds the r4 shape: a failed v1 request and
  no action. It requires "legacy v1 edit evidence requires a fresh task
  state", 0 POSTs and an unchanged file.
- **A2.** `docs/llama-adapter.md`'s application paragraph now states S033
  items 2 and 4. Application's own pre-start checks cancel, and `start`'s
  run-level guards leave the action prepared and resumable. The Windows
  ARCHIVE/NORMAL exclusion is named. The file's 141 CRLF lines are preserved,
  and the new lines are LF like their neighbours.
- **A3.** New `TaskReader::legacy_edit_requests` (`src/workspace.rs`) is a
  read-only count of saved `admitted_task_edit_v1:` requests. New
  `edit_review::no_edit_review_line` uses that count. `task-edit-review` now
  prints "No v2 edit session or edit action is recorded. This task holds N
  historical v1 whole-file edit request(s) and no edit action; `task-view` and
  `run-status` describe them."
  - With no v1 requests, the old sentence stays.
  - `--json` still prints `null`, now documented as "no v2 session and no
    edit action".
  - Observed on a fresh r4 copy with the gated binary: the new sentence
    printed, `null` printed, `task-view --json` was still equivalent to the
    frozen projection, and the original r4 hash was unchanged.
  - Recorded in S033 and `docs/llama-adapter.md`.
- **A4.** The v1 helpers now write the removed executor's rows:
  - a granted permission first;
  - the v1 request through prepared, started and settled, at the run's
    ordinal, with provider `shuttle-llama:<digest>` and grant revision 2;
  - the action inserted with the executor's own SQL, in one transaction with
    marking the reply applied (`admitted_task_edit:<action>`);
  - ID `<run>/admitted-write/<permission>`, the pre-edit `input_hash`, and the
    executor's artifact.

  The one stated reduction: the saved serialized request keeps only its
  `protocol` field. The historical test now also checks `task-edit-review`
  and `task-view` for the bytes.

  Two fixture shapes were tried and failed first. They are recorded because
  they show why the action-only test must stay synthetic:
  - `journal.prepare` refuses while the v1 reply is unapplied, which is why
    the executor used its own transaction.
  - A started-then-recovered unknown v1 action is refused earlier by the
    generic "unknown completion blocks new work" guard.

  Every real v1 action has its v1 request, and the request check refuses
  first. So the action-only test keeps a synthetic `failed` action with no
  request, and the helper's comment says so.
- **A5.** `CONIN$` and `CONOUT$` are now reserved. The table gains
  `a/CONIN$` and `a/conout$.txt`, and the pass-through list gains
  `src/conin.rs` and `COM⁴`. `COM⁴` was observed as an ordinary file here,
  and Python agrees. The comment now records what was observed and calls the
  list defensive. S033's Chunk 6 clarification is updated, and "exhaustive"
  is corrected in `docs/implementation-status.md`.
- **A6.** `a_linked_parent_directory_never_yields_outside_bytes` now runs
  twice: once with a byte-identical outside copy and once with a differing
  one. Both runs assert "symlink path is not allowed". New
  `a_linked_workspace_root_is_refused` links the root back to the unchanged
  real workspace. The link helpers are shared.
- **A7.** The action-only test asserts "legacy whole-file action requires a
  fresh task state".
- **A8.** S033's opening now reads: Chunks 1–5 implemented 2026-09-16 to
  2026-09-25; 2a had no reviewer; audit fixes applied; re-check pending.

**Mutation checks.** Each was restored by copying the saved file back, which
also updates mtime, and every file was compared with `cmp`.

| Mutation | Result |
| --- | --- |
| M2: the v1 request check deleted (alone) | Only the new A1 test fails. It was the surviving mutant in the audit. |
| M1 + M2: both legacy checks deleted | Both v1 refusal tests fail; the edit runs. |
| `CONIN$`/`CONOUT$` removed from the list | The table fails at `a/CONIN$`. |
| `no_edit_review_line` forced to the old sentence | The A1 test fails, printing "No admitted edit has been attempted…". |
| Symlink and reparse clauses of `check_absolute_path` disabled | Both link tests fail. Other layers still refuse, with different messages: "snapshot input must be a regular file" for `src`, and "Declared inputs differ" for the root. |

The last row shows that the tests now pin the link check itself.

**A first Docker run failed and is kept:**
`.shuttle/chunk-6-audit-fixes-docker-linux-run1-clippy-unused-mut.log`.
Clippy `-D warnings` flagged an unused `mut` in the new
`record_v1_whole_file_action`. Its tests had all passed (266). The `mut` was
removed, local fmt and Clippy passed, and both gates then ran from scratch.

**Gates, final tree** (`c6b4302` plus the uncommitted Chunk 6 tree with
audit fixes A1–A8):

| Gate | fmt | check | Clippy `-D warnings` | `preserve_order` | `cargo test --locked --no-fail-fast` |
| --- | --- | --- | --- | --- | --- |
| Windows, native MSVC, Rust 1.98.0, `-j 4`, repository `target\` | pass | pass | pass | none | **261 passed, 0 failed, 10 ignored** |
| Docker Linux, `shuttle-qualification:local` `sha256:7a49e23f…cb86b`, historical run shape | pass | pass | pass | none | **266 passed, 0 failed, 10 ignored** |

Both are +2 over the audit's re-observed gates: the A1 test and the root-link
test. The other changed tests replace existing ones. The legs ran one after
the other, Docker first. Logs:
- `.shuttle/chunk-6-audit-fixes-windows-gate.log`
- `.shuttle/chunk-6-audit-fixes-docker-linux.log`, with the command on its
  first line and the image ID on its second

Script: `.shuttle/chunk-6-audit-fixes-docker-gate.sh`. The `llama_adapter`
flake did not recur.

**Next task:** the focused re-check of these diffs, in a fresh session
(Sonnet : high or Opus : high). Priorities:
1. The A4 helpers against `d0bd549`'s `run_admitted_task_edit`.
2. The A3 query on journals that predate migrations 0022/0023.
3. `docs/llama-adapter.md` against S033 items 2 and 4.

Once the re-check passes and the operator accepts, Chunk 6 is done. Then
Chunk 7, after the `thinking`/`max_tokens` decision.

### Focused re-check of the audit fixes (2026-09-25)

Performed by Claude (Opus, high effort) in a fresh local session on the
operator's Windows machine. It did not find or fix A1–A8. It covered the three
priorities above on the uncommitted tree (`c6b4302` plus Chunk 6 plus A1–A8).
No source, test or doc file was changed. Two probes were applied temporarily,
then restored from saved copies and compared with `cmp`. `git diff --stat`
again shows the same 8 files, +817/−16.

**Verdict: passed on all three priorities, with two Low wording findings
(RC-1, RC-2).** Neither is a safety defect. RC-1 is a display claim stronger
than observed behavior, so it falls under the exit gate. Both are one-line
fixes, and a wording-only fix needs no further independent review.

#### Priority 1: A4 helpers against `d0bd549`'s `run_admitted_task_edit`

Every field was compared with the removed executor and with the current
`prepare_model`, `start_model` and `finish_model` in `src/requests.rs`.
- Matching fields:
  - provider `shuttle-llama:<digest>` (the v1 mode's `with_mode` prefix);
  - purpose `admitted_task_edit_v1:<context>:<permission>`;
  - request `input_hash` = the permission's snapshot, grant revision 2;
  - ID `<run>/request/<model_responses>` with the counter bumped;
  - `finish_model`'s `state`/`result_json`/`applied`/`application` columns;
  - the reply-applied update and the action insert in one transaction,
    with `admitted_task_edit:<action>`;
  - action ID `<run>/admitted-write/<permission>`, `input_hash` = the pre-edit
    snapshot, revision 2;
  - `expected_hash` = blake3 of the file (the snapshot hash
    `validate_workspace_edits` compared);
  - the artifact `{permission_id, paths, post_snapshot}` over the plan's
    snapshot capture.
- `timeout_ms` 120,000 is the profile default.
- The `serialized_request` reduction is stated in the comment.
- Omitted: `finish_model`'s `INSERT OR IGNORE` of provider artifacts into
  `artifacts`. It is unlinked and changes no check.

- **RC-2 (Low; fixture comment overclaims fidelity).**
  `record_v1_edit_request` says it saves the request "through the same states
  `prepare_model`, `start_model` and `finish_model` gave it", and the A1 test
  calls its fixture "the retained r4 shape". But `finish_model` with an
  error also calls `transition(paused, "Model request failed; attempt and
  available usage retained")`, and the helper does not. r4 is paused with
  exactly that reason (read from a copy).
  - Probe P1: the pause, and its `transitions` row, added to the helper for
    failed results. Both v1 refusal tests still pass with the same messages.
    So the missing pause does not change what the tests prove. The legacy
    check runs before any phase check in `open_admitted_edit_session`.
  - *Fix:* add the pause (more faithful, and verified to pass), or say in the
    comment that the run is left `ready`.

**M2 re-confirmed.** With the v1-request `ensure!` deleted from
`open_admitted_edit_session`:
- As written (run left `ready`): only the A1 test fails. The v2 edit runs to
  a succeeded `PatchWorkspaceFiles` action, so the mutant is strongly killed.
- With P1's pause: the A1 test still fails, on the message assertion. The
  paused run is still refused with 0 POSTs, by "edit session run is not
  ready". So on a real r4-shaped journal, the legacy check has a second guard
  behind it.
- Both files restored and `cmp`-identical to the saved copies.

#### Priority 2: the A3 query on journals before 0022/0023

- `legacy_edit_requests` reads only `model_requests`, which migration 0005
  created. 0005's backfilled placeholder rows are valid JSON, so
  `json_extract` cannot fail on them. The `LIKE … ESCAPE` pattern matches the
  `admitted_task_edit_v1:` prefix literally.
- `load_session_review`, which `main.rs` calls first, already probes
  `sqlite_schema` and `pragma_table_info` for the 0022 table and the 0023
  column.
- **Observed on a copy of r4, which is at migration 21, before both:**
  - `task-edit-review` printed "No v2 edit session or edit action is recorded.
    This task holds 1 historical v1 whole-file edit request(s)…", exit 0.
  - `--json` printed `null`, exit 0.
  - The original `journal.sqlite` still hashes `0CACBD08…082F`, and no
    sidecars were created next to it.
- Not observed: a journal before 0005 would now make `task-edit-review` fail
  on the missing table. Such a journal cannot hold a task.

- **RC-1 (Low; the display sentence claims more than `task-view` shows).**
  The new sentence (`src/edit_review.rs` `no_edit_review_line`) ends
  "`task-view` and `run-status` describe them."
  - On the r4 copy, `run-status` does. It lists the request with its
    `admitted_task_edit_v1:` purpose and "transport response exceeds 64 KiB
    bound".
  - `task-view --json` does not. It shows only `phase: paused`, the reason
    "Model request failed; attempt and available usage retained", and the
    direction "Permission is saved. Request the bounded patch; it will
    revalidate before writing." Nothing identifies a v1 edit request.
  - The docs (S033's A3 clarification, `docs/llama-adapter.md`) do not repeat
    the claim. Only the CLI string, and this plan's A3 note, carry it.
  - *Fix:* "…; `run-status` lists them." The test asserts only the count
    phrase, so it is unaffected.

#### Priority 3: `docs/llama-adapter.md` against S033 items 2 and 4

Agrees in substance.
- **Item 2:** own checks cancel, and a cancelled action is never resumed. The
  four run-level guards inside `start` are named identically and leave the
  action `prepared`. Failure after the marker is `unknown`.
- **Item 4:** exact equality with the predicted snapshot, with the
  Windows-only ARCHIVE and NORMAL exclusion on the patched files. Every other
  attribute and every unpatched file stays exact.
- Omissions, not overclaims: that a cancel pauses the run, and that a guard
  failure leaves the run unchanged.
- *Nit:* "it resumes once the condition clears" can read as automatic. S033
  says application *can* resume. The doc says "4b-4d" where S033's heading
  uses an en dash.
- Line endings: HEAD had 138 CRLF lines, and all are intact. The 3 new CRLF
  lines are Chunk 6's streamed-section note inside a CRLF region. The A2
  lines are LF among LF neighbours. The fix record's "141 CRLF lines are
  preserved" is accurate for the working file.

#### Out of scope, flagged

On r4, `task-view`'s direction still says "Request the bounded patch". But
v2 `task-edit` refuses that journal ("legacy v1 edit evidence requires a
fresh task state"). This predates Chunk 6, and the frozen r4 projection
records the same text. It is a misleading operator direction on legacy
journals, not a safety issue. It is worth a small change to the direction
logic later, noting that it changes the r4 projection equivalence check.

#### Gate, final tree (unchanged by this re-check)

| Gate | fmt | Clippy `--all-targets -D warnings` | `cargo test --locked --no-fail-fast` |
| --- | --- | --- | --- |
| Windows, native MSVC, `-j 4`, repository `target\` | pass | pass | **261 passed, 0 failed, 10 ignored** |

This matches the audit-fixes record. Docker Linux was not re-run, because
nothing changed.

**Next task:** fix RC-1 and RC-2 (wording, or the pause for RC-2). Then the
operator accepts Chunk 6, and S033's opening "re-check pending" clause is
updated. Then Chunk 7, after the `thinking`/`max_tokens` decision.

#### Re-check fixes applied (2026-09-25)

Applied in the re-check session, at the operator's request. Both fixes are
wording or fixture fidelity only. As the verdict above says, they need no
further independent review.
- **RC-1.** `no_edit_review_line` now ends "…; `run-status` lists them." On
  the r4 copy, the gated binary prints the corrected sentence.
- **RC-2.** `record_v1_edit_request` now pauses the run for a failed result
  with `finish_model`'s reason and a `transitions` row, so the A1 fixture is
  the r4 shape it claims to be. The doc comment says so. The historical
  success test is unaffected, since its request succeeds.
- **M2 re-run on the final fixture:** only the A1 test fails, on its message
  assertion; the paused run is refused by "edit session run is not ready".
  `src/edit_session.rs` was restored and is `cmp`-identical.

| Gate | fmt | Clippy `--all-targets -D warnings` | `cargo test --locked --no-fail-fast` |
| --- | --- | --- | --- |
| Windows, native MSVC, `-j 4`, repository `target\` | pass | pass | **261 passed, 0 failed, 10 ignored** |

Docker Linux was not re-run for these two changes.

**Next task:** the operator accepts Chunk 6, and S033's opening "re-check
pending" clause is updated. Then Chunk 7, after the
`thinking`/`max_tokens` decision.

## Chunk 6 acceptance (2026-09-25)

The operator accepted Chunk 6 on 2026-09-25, in chat. That covers the
consolidation, audit fixes A1–A8 and re-check fixes RC-1 and RC-2. It rests
on the evidence recorded above:
- the independent Opus : high audit;
- the fixes and their mutation checks;
- a focused re-check in a separate session;
- passing gates.

The last gate on both platforms was on the A1–A8 tree: Windows 261 and Docker
Linux 266 passed, none failed. After RC-1 and RC-2, only Windows was re-run
(261 passed). Docker Linux was not re-run for those two changes. RC-1 and
RC-2 were applied in the re-check session and had no further independent
review; the operator accepted on that basis. Nothing was committed, at the
operator's instruction.

S033's opening and the `docs/implementation-status.md` Chunk 6 entry now
record that the audit, re-check and acceptance are done.

Limits carried forward:
- the edit path's own per-component link check is untested in isolation, and
  it is redundant with the handle-identity and admitted-hash checks (A6);
- Chunk 5's hand-written escape list, and CLI JSON prints outside
  `json_terminal_safe`;
- the misleading `task-view` direction on legacy v1 journals (flagged in the
  re-check, out of scope);
- the plan-summary-in-edit-context candidate, and `open_admitted_edit_session`
  not checking that turn 0 composes.

This does not satisfy Chunk 7's live qualification.

### Next task

- The operator decides the qualification profile's `thinking` and
  `max_tokens`.
- Then Chunk 7, the live emCP qualification.

## Chunk 7 profile decision (2026-09-25)

The operator decided in chat: **the qualification profile keeps r4's settings,
`thinking: false` and `max_tokens: 512`.** Capture it with plain
`shuttle llama-profile`, without `--thinking`. Capture always writes
`max_tokens` 512, so no hand-editing is needed or allowed.

Reasons:
- **A controlled comparison.** Chunk 7 repeats r4 to show that the v2
  protocol fixes r4's failure. The saved emCP profile
  (`.shuttle/emcp-worker-b10278.json`) and r4's saved requests both show
  thinking off and 512. Changing only the protocol keeps the result
  attributable.
- **Thinking shares the output budget.** Earlier live runs on the same model
  used 14 output tokens per trivial step with thinking off, and 83–114 with it
  on. A truncated patch turn is a decoder rejection. That closes the session
  and costs a fresh task state. The only thinking-on attempt (04) exceeded
  the transport bound. The only full live success (07) ran with thinking off.
- **512 has headroom.** The patch call is estimated at about 150–190 tokens,
  and r4's planning reply used 146. The context check is far inside the
  worker's reported `n_ctx` of 65,280 (24,000 + 512 + 4,096). The strict bound
  of 573 or fewer is still an estimate. Only live usage settles it.

**Fallback rule, fixed in advance:** a truncated patch turn is one where
`run-status` shows output tokens at or near 512 and a decode rejection. Only
that failure justifies raising `max_tokens`, to 1024. Do it in a new task
state, preferably through a new `llama-profile --max-tokens` flag rather than
a hand-edited profile. Record the attempt as a failed qualification. Any other
failure is a finding about the protocol and does not change the profile.

Out of scope for Chunk 7: whether thinking improves patch quality. That is a
separate experiment for a separate run after qualification.

### Next task

Chunk 7, the live emCP qualification, in a **fresh session** (operator's
choice). It should work from this plan's Chunk 7 procedure and pass criteria.
It must not start, stop or reconfigure the worker. It must not commit to emCP
or create a worktree there. It must keep r4 untouched. Capture or validate the
profile against the running worker first, and stop if the worker is not the
recorded Qwen3.5-4B Q4_K_M model. The operator performs the grant, reviews the
diff and makes the accept/reject decision. Chunk 6 is committed as
`556eaab` (operator, 2026-09-25); Chunk 7 runs against that commit.

Inputs for the fresh session. Everything is local; nothing needs fetching.
- **Procedure and pass criteria:** "Chunk 7 — Repeat the real emCP
  qualification" under "Work chunks". Its GPT model labels are historical.
  The Step 0 table supersedes them: Sonnet : medium does the primary work, and
  a separate Opus : high session does the final evidence review.
- **Workspace:** `C:\dev\agentic\emCP`, baseline `a6527896` on `main`
  (`docs/manual-qualifications/emcp-2026-09-14.md`). Observed 2026-09-25: HEAD
  is still `a652789`. Two untracked directories, `.indexing-test-v9QZXJ/` and
  `.retrieval-test-GHo3CF/`, look like emCP test leftovers. The tree is not
  clean until the operator resolves them. Shuttle must not delete them.
  They are `mkdtemp` roots from emCP's `tests/integration/indexing.test.ts`
  and `retrieval.test.ts`, dated 2026-08-27, so they predate the baseline.
  The operator deleted both on 2026-09-25, and `git status --short` is now
  empty at `a652789`.
- **Objective and constraints to repeat:** r4's saved intake. The objective
  is "Correct AGENTS.md so its testing guidance accurately distinguishes the
  opt-in live embedding test from the MCP stdio end-to-end test." The
  constraint text is in r4's `task_intakes` row. Read it from a copy or with
  SQLite's `immutable=1`, never by opening r4 through Shuttle.
- **Verification plan:** `.shuttle/emcp-agents-live-test-plan.json`: one
  check, `npm run check`, with inputs `AGENTS.md` and `package.json`.
- **Worker:** user-managed Qwen3.5-4B Q4_K_M on `127.0.0.1:8080`. The old
  profile is `.shuttle/emcp-worker-b10278.json`. Re-capture a fresh profile
  (thinking off) against the running worker. Do not reuse the old file blindly.
- **Checklist:** `docs/manual-qualifications/manual-admitted-task-qualification.md`
  and the r4 record `docs/manual-qualifications/emcp-2026-09-16-r4.md`.
- **New state directory:** a fresh `.shuttle/` directory outside the emCP
  checkout, never r4's.

## Chunk 7 result and S033 closure (2026-09-25)

Record: `docs/manual-qualifications/emcp-2026-09-25-chunk7.md`, including the
follow-up diagnosis and the Opus : high final evidence review.

- Attempt 7g met pass criteria 1–5 once. The patch response was 1,367 bytes,
  with 221 of 512 output tokens. One exact hunk was applied. Every required
  item is inspectable, and Shuttle made no endpoint, commit or worktree
  change. The full check passed. The operator rejected the patch, which
  invents `npm run test:mcp`. Attempt 7 was the same deterministic sample and
  never reached verification.
- The review found no protocol defect, no raised limit and no weakened gate,
  including plan `e`'s environment. It put the wrong patch down to the edit
  context (only permitted files, no plan summary) and the model's read
  strategy.
- **Operator decision (K2 = E):** Chunk 7 criterion 6 and intended outcome 8
  are amended as marked in place above. S033 is closed on the protocol result.
  The amended texts and the limits of closure are in S033's closure
  clarification in `docs/decisions.md`.

### Next task

- Step 9, the Windows Terminal and native Linux manual-usability records.
  This is operator only and outside S033's exit.
- K3, the executor fixes: a static process-spec check at preflight, the
  `not_prepared` status label, the verbatim cwd at the spawn boundary, and
  the Windows environment baseline in the guide.
- K4: replace or retire `.shuttle/emcp-agents-live-test-plan.json`.
- K5 and K6, only if chosen: an edit-context-sufficiency increment (S034),
  then a comparable live run.

## K3 result: executor fixes (2026-09-25)

Shuttle code only, uncommitted. No stored byte, migration, bound or grant
changes.

- **K3a.** `validate_process_spec` in `src/process.rs` holds the static launch
  checks: platform, limits, absolute executable, the Windows `.exe` rule, the
  executable hash, the working directory, and argument and environment
  bounds. `ProcessExecutor::validate` calls it after its grant and input
  checks, so the only reorder is that limits are now checked after the input
  precondition. `preflight_intake` calls it for every unwaived check, so a
  `.cmd` plan is refused before admission. `task-verify-status` reports
  `not_prepared` for a binding with no action row; `unknown` stays for a
  prepared or started action with no receipt.
- **K3b.** `win32_directory` in `src/process/windows.rs` strips `\\?\` from the
  spawn-time `lpCurrentDirectory` only for a `VerbatimDisk` path whose
  components Win32 would not rewrite (no `.`/`..`, trailing dot or space,
  reserved device name or invalid character) and whose length is at most 258.
  No identity uses that string.
- **K3c.** Not done. Turning a post-start re-validation failure into a
  `Failed` observation changes what a failure after the started marker records,
  so it needs its own decision.
- **K3d.** `docs/process-execution.md`, `docs/verification.md`, and the
  (gitignored) manual guide's Windows environment baseline.

Tests: three simplifier unit tests; a Windows spawn test where the child and
its own `cmd.exe /d /c cd` see the plain root, with no "UNC" on stderr and the
authorization still bound to the verbatim root; a preflight test for a missing
cwd, a wrong hash, and (Windows) a `.cmd` executable, with a waived check not
blocking; and a CLI test where a removed working directory gives
`not_prepared`. Mutation checks: removing each fix fails its tests.

### Gate

| Gate | fmt | Clippy `-D warnings` | `cargo test --locked` |
| --- | --- | --- | --- |
| Windows, native MSVC | pass | pass | **267 passed, 0 failed, 10 ignored** (`.shuttle/k3-windows-gate.log`) |
| Docker Linux | not run: Docker Desktop was not running | | |

### Next task

- Run the Docker Linux leg (`.shuttle/chunk-6-audit-docker-gate.sh`).
- A focused review, then operator acceptance and commit.

## Why this increment exists

The retained task at
`.shuttle/manual-emcp-2026-09-15-r4` successfully exercised intake, preflight,
admission, read-only planning, path permission, and the beginning of editing. Its
planning request succeeded with 1,569 input tokens and 146 output tokens. The
permitted patch request reached the pinned worker and received HTTP 200, but its
raw streamed response crossed Shuttle's 65,536-byte transport bound before a
complete response and usage record arrived. Shuttle paused the task without
applying a workspace action.

That result exposed a protocol problem rather than an emCP problem. The current
admitted-edit protocol asks the model to return every byte of every replacement
file as JSON integers. `AGENTS.md` is approximately 11.6 KiB, so a small paragraph
change becomes an output far larger than the change, the model's configured output
budget, and the retained transport envelope.

The next protocol should make response size proportional to the edit while keeping
the existing admission, permission, durability, freshness, review, and verification
boundaries.

## Intended outcome

Shuttle can safely apply one bounded set of exact text hunks to one or more
permitted existing UTF-8 files. A model can inspect bounded portions of a larger
file, return only the changed text plus exact preimage anchors, and complete the
original emCP `AGENTS.md` objective without returning the whole file.

The implementation is complete only when:

1. The retained `r4` failure remains readable and is never reinterpreted under the
   new protocol.
2. A new protocol identity cannot replay or decode a v1 whole-file response as a
   v2 hunk patch.
3. Every patch is bound to the current admitted snapshot, explicit path grant,
   whole-file preimage hashes, and exact old text.
4. All hunks are validated and final file images are constructed before the first
   filesystem write.
5. A changed file, missing or ambiguous anchor, overlapping hunk, invalid UTF-8,
   unauthorized path, link/reparse target, no-op patch, or exceeded bound fails
   before any write.
6. A started filesystem action remains unknown after interruption and is never
   automatically replayed. The implementation must not claim multi-file filesystem
   atomicity.
7. Successful edits still require explicit re-admission, verification, review,
   accept/reject, and finalization through the existing workflow.
8. The original emCP documentation task completes in a fresh task state with a
   compact response comfortably inside the configured token and transport bounds.
   **Amended 2026-09-25 (operator, Chunk 7):** the original task runs in a
   fresh task state to an explicit human decision, with a compact edit response
   comfortably inside those bounds. Task completion is not claimed. See "Chunk
   7 result and S033 closure".

## Scope and non-goals

In scope:

- existing declared regular UTF-8 files;
- exact replacement/deletion hunks against a content-addressed preimage;
- insertion expressed initially by replacing a unique anchor with that anchor plus
  inserted text;
- multiple non-overlapping hunks and multiple permitted files;
- bounded read/search context for files whose relevant region is not present in the
  initial preview;
- versioned provider, journal, status, review, documentation, and qualification
  behavior;
- Windows and Linux tests for path, newline, Unicode, crash-boundary, and replay
  behavior.

Out of scope for this increment:

- binary files;
- creating, deleting, or renaming files;
- symlink, junction, or reparse-point targets;
- semantic AST transforms or language-server integration;
- automatic conflict resolution, fuzzy patching, or offset guessing;
- automatic rollback after the first filesystem effect;
- increasing limits as the primary solution;
- changing the persistent-terminal/TUI lifecycle.

Those exclusions should remain explicit extension points rather than accidental
protocol behavior.

## Proposed v2 contract

The provider-facing patch should be compact, human-readable UTF-8 text rather than
an integer for every byte. One candidate wire shape is:

```json
{
  "files": [
    {
      "path": "AGENTS.md",
      "expected_file_hash": "<BLAKE3 from admitted snapshot>",
      "hunks": [
        {
          "old_utf8": "# Run all tests\nnpm run test",
          "new_utf8": "# Run the offline suite, including MCP stdio end-to-end coverage\nnpm run test"
        }
      ]
    }
  ]
}
```

Normative rules:

- `old_utf8` is nonempty and must occur exactly once in the admitted file preimage.
- `new_utf8` may be empty. JSON decoding is the only escape-decoding pass; Shuttle
  never appends a newline, expands a second escape layer, or repairs model output.
- The complete current file must match `expected_file_hash` and the admitted
  snapshot before patch preparation and immediately before writing.
- All hunk ranges are resolved against the same immutable preimage. They must not
  overlap. Application occurs from the highest resolved byte offset downward.
- The generated final bytes must be valid UTF-8, differ from the preimage, stay
  within explicit per-file and aggregate limits, and have their expected postimage
  hashes persisted in the action intent.
- All paths must be normalized, permitted, declared, existing regular files under
  the canonical workspace root. No model-supplied absolute path is accepted.
- The decoded patch and the serialized durable reply are independently bounded.

Before implementation, the architecture chunk must decide and record concrete
limits for file size, files per patch, hunks per file, aggregate old/new text,
read observations, model turns, serialized reply size, and transport capture.
Bounds should be justified from existing journal and verification limits rather
than chosen only to make the qualification pass.

### Bounded context acquisition

Partial writes are not sufficient if the model cannot see the exact target text.
The editing protocol should therefore permit a small, durable, read-only context
phase after path permission and before the patch:

- `read_task_text(path, start_line, line_count)` returns a bounded exact UTF-8
  excerpt, file hash, line range, newline metadata, and truncation status.
- `find_task_text(path, literal)` returns bounded exact-match locations plus small
  surrounding excerpts. It performs literal search only; no regex or shell.
- `record_task_patch(files)` ends the context phase and proposes the hunk set.

Each response uses exactly one tool. Reads are limited to currently permitted
paths, revalidate the admitted snapshot before access, produce bounded durable
observations, and consume a fixed edit-session turn budget. They grant no write or
command capability. The model cannot patch until it returns `record_task_patch`.

If the architecture review finds that this session cannot be added without
entangling unrelated fixture actions, split it into a dedicated admitted-edit
session ledger. Do not smuggle file reads into transient in-memory history.

### Transport recommendation

Evaluate a non-streamed, bounded `application/json` response for the v2 patch tool.
The terminal does not display model tokens, and token-by-token SSE metadata can be
larger than the actual tool arguments. The exact response body can still be read
with a hard byte cap, hashed, stored, decoded once, and recorded with provider
usage. Preserve the existing one-POST/no-retry rule and pre/post worker identity
checks.

If streaming is retained, the design must derive a safe raw-capture allowance from
the configured output budget and prove that a maximum-size valid v2 reply fits. A
larger unexplained constant is not sufficient.

## Durable representation and compatibility

Do not mutate the meaning of historical JSON variants. Keep the legacy
`WriteWorkspaceFiles`/whole-file structures readable for old journals, but stop
generating them for new admitted tasks. Add distinct v2 types, such as:

- `WorkspaceTextHunk`;
- `WorkspaceFilePatch`;
- `ToolCall::PatchWorkspaceFiles`;
- a distinct model decision variant for the admitted hunk patch.

Use a new provider protocol identity and purpose identity. A saved v1 request or
response must fail closed under v2 rather than being upgraded in place. The failed
`r4` run remains paused evidence; the final qualification uses a new state
directory.

The durable action should contain enough information to audit and inspect the
operation without rereading mutable source:

- permission and admission identities;
- path and whole-file preimage hash;
- exact old/new hunk text or bytes;
- resolved byte ranges;
- expected postimage hash;
- ordered patch-set identity;
- explicit bounds/protocol version.

The write stage validates every target and constructs every postimage before
marking the action started. It then marks the action started before the first
filesystem effect, as today. Writes may be individually hardened with temporary
files and replacement where portable and metadata-safe, but the documentation must
continue to state that a multi-file write is not one filesystem transaction.

## Work chunks and historical model allocation

The sequence is intentionally serial at contract boundaries. Later coding chunks
may run independently only after their prerequisite contract and types are merged.
Each chunk should receive only the listed context pack plus the diff from its
immediate predecessor. The model labels in this section are historical Codex
assignments; Step 0 must replace them with Claude choices before future work starts.

### Chunk 0 — Freeze the failure and write the contract

Completed 2026-09-16: [S033](../docs/decisions.md#s033--apply-bounded-exact-text-hunks-under-a-versioned-admitted-edit-contract)
records the contract and primary/Terra review agreement. The
[r4 failure projections](../docs/manual-qualifications/emcp-2026-09-16-r4.md)
are frozen; no production implementation was made.

Primary: `gpt-6-astra:high`

Review: `gpt-5.6-terra:high`

Context pack:

- this plan;
- `docs/decisions.md` S015, S029, and S032;
- `docs/llama-adapter.md` fixed profile, bounds, and admitted-task patch sections;
- `docs/live-repair.md` exact-byte and response-bound limitations;
- the `r4` `task-view --json` and `run-status` projections, not raw model content;
- focused portions of `src/journal.rs`, `src/llama.rs`, and `src/workspace.rs`.

Deliverables:

- ADR S033 defining v2 semantics, limits, transport choice, compatibility, and
  crash behavior;
- a protocol examples table covering replacement, deletion, insertion-by-anchor,
  multiple hunks, and multi-file validation;
- an explicit threat/failure matrix;
- no production implementation.

Exit gate: two-model agreement that the contract is deterministic, bounded,
backward-compatible, and implementable without claiming filesystem atomicity.

### Chunk 1 — Build the pure patch planner

Implemented 2026-09-16: v2 patch domain types and the side-effect-free planner
are in `src/journal.rs` and `src/workspace.rs`; table-driven coverage is in
`tests/workspace_text_patch.rs`. The `gpt-6-astra:high` invariant review approved
the implementation after schema and boundary-test corrections. `cargo fmt --check`,
`cargo check --locked --tests`, and warnings-denied Clippy pass. Runtime execution
of the focused test target remains pending because this host has no `link.exe`;
`cargo test --locked --test workspace_text_patch` cannot link the Windows binary.
Do not treat the runtime-test exit gate as passed until it runs in a complete MSVC
or Linux toolchain.

Primary: `gpt-5.6-terra:high`

Review: `gpt-6-astra:high` focused only on invariants and adversarial cases.

Context pack:

- accepted S033;
- `src/journal.rs` action types;
- the current `validate_workspace_edits` and `write_workspace_edits` helpers;
- focused task-planning tests.

Deliverables:

- v2 patch domain types;
- a side-effect-free function that validates paths/hunks against admitted
  preimages, resolves unique byte ranges, rejects overlap/ambiguity/no-op, creates
  final bytes, and calculates postimage hashes;
- table-driven tests for LF/CRLF, Unicode boundaries, repeated anchors, empty
  replacement, insertion-by-anchor, reordered hunks, overlap, size limits, and
  deterministic patch identity;
- legacy type decoding retained.

Exit gate: exhaustive focused tests and a review showing the pure planner performs
no filesystem or journal mutation.

### Chunk 2 — Add bounded admitted-file reads

Primary: `gpt-6-astra:high`

Implementation follow-through: `gpt-5.6-terra:high`

Context pack:

- accepted S033 and Chunk 1 types;
- `AdmittedTaskContext` preview construction;
- request ledger and model-context history code;
- write-permission lookup and source-snapshot revalidation helpers.

Deliverables:

- versioned admitted-edit session state;
- bounded literal find and line-range read operations scoped to granted paths;
- exact durable observations with hashes/ranges/truncation metadata;
- per-operation and cumulative read/turn budgets;
- tests proving changed inputs, ungranted paths, non-UTF-8 files, traversal,
  links/reparse points, oversized reads, and exhausted turns fail closed;
- restart/replay tests proving saved observations are reused without silently
  rereading changed source.

Exit gate: the model can acquire an exact excerpt from beyond the initial 1 KiB
preview without receiving a write or command capability.

### Chunk 3 — Implement the local-model v2 wire protocol

Primary: `gpt-5.6-terra:high`

Review: `gpt-5.6-sol:high`

Context pack:

- accepted protocol schema and bounds;
- `src/llama.rs` admitted planning/editing paths;
- `src/llama/stream.rs` only if streaming remains;
- local-model mock HTTP tests.

Deliverables:

- new provider/protocol identity;
- strict schemas for bounded reads/finds and `record_task_patch`;
- compact UTF-8 hunk decoding with exactly one JSON escape pass;
- selected bounded transport implementation with exact artifact capture;
- matching-model, single-choice, single-tool, usage, content-type, completion, and
  pre/post identity validation;
- malformed, extra-field, wrong-tool, oversized, incomplete, and hostile-output
  tests;
- proof that v1 saved requests cannot enter the v2 decoder.

Exit gate: a mock response representing the emCP paragraph edit remains well under
the request, token, reply, and transport bounds.

### Chunk 4 — Integrate durable preparation and filesystem application

Primary: `gpt-5.6-terra:high`

Review: `gpt-6-astra:high`

Context pack:

- Chunks 1–3;
- `run_admitted_task_edit`;
- action and request transactions;
- source capture, path safety, readmission, evidence, offer, and acceptance code.

Deliverables:

- v2 edit-session orchestration through the existing durable model ledger;
- atomic journal commit of successful patch reply plus prepared action;
- complete prevalidation/postimage construction before action start;
- immediate freshness and path-type revalidation before writing;
- exact postimage/action artifact persisted after success;
- unchanged re-admission, verification, offer, decision, and finalization gates;
- fault-injection tests before action preparation, after preparation, after start,
  between file writes, and before completion.
- size-aware read commit (Chunk 3 review R1): after every committed read, the
  next request, patch-only turn included, must still fit the 24,000-byte
  request and durable-context bounds. At read commit in
  `finish_admitted_text_read`, measure the exact next context size
  (deterministic). If it would not fit, shorten the excerpt through the existing
  allowance passed to `observe_text` and mark it truncated. No saved
  observation is dropped and no limit is raised (S033). Needs an S033
  clarification before implementation. Two decisions must be recorded first:
  (a) if even a minimal excerpt cannot fit, the read is refused in a way that
  keeps a patch turn available, where today a read failure closes the session
  under G2; (b) a shortened excerpt consumes its actual bytes from the
  cumulative read budget.
- boundary tests for R1: three maximum reads on ordinary text still leave room
  for a patch turn; a backslash-dense file no longer strands the session; the
  refused-read case; and the budget accounting in (b).
- offer/accept parity pin (Chunk 3 review, deferred here on purpose): once R1
  fixes what "a read can still succeed" means, add one test that whenever the
  adapter's `AdmittedEditTurnContext::reads_available()` offers a read tool, the
  journal's read availability check (`ensure_read_turn_available` plus the
  remaining-byte check in `finish_admitted_text_read`) also accepts a read, over
  every (turn, read count, remaining bytes) combination the budgets allow. Write
  it against the final definition, in the same change that alters it, so it is
  not written twice.

Exit gate: every interruption has an explicit replay rule; no started write is
automatically retried or described as atomic; no sequence of permitted reads can leave a session with no representable patch
turn.

### Chunk 5 — Update task projections and operator review surfaces

Primary: `gpt-5.6-terra:medium`

Review: `gpt-5.6-terra:high`

Context pack:

- v2 durable types and artifacts;
- task reader/projection code;
- existing task workspace, write-status, and offer-review renderers.

Deliverables:

- bounded summaries of file/hunk counts, paths, pre/post hashes, and limitations;
- review output that shows the exact compact hunks without dumping entire files;
- clear paused/unknown/ambiguous-anchor messages;
- compatibility rendering for historical whole-file actions;
- snapshot tests at narrow and normal terminal sizes.

Exit gate: an operator can understand exactly what text changed and why a patch was
blocked without reading raw provider output.

This chunk does not change the terminal open/close lifecycle.

### Chunk 6 — Consolidate verification, documentation, and compatibility

Primary: `gpt-5.6-terra:medium`

Independent audit: `gpt-5.6-sol:high`

Context pack:

- the completed diff;
- S033 and affected public docs;
- focused and integration test inventories.

Deliverables:

- focused tests for all contract invariants;
- end-to-end mock-worker tests for read/find/patch/restart;
- Windows path and CRLF coverage plus Linux coverage;
- old-journal read tests and protocol-identity rejection tests;
- updated `docs/llama-adapter.md`, `docs/decisions.md`,
  `docs/implementation-status.md`, manual qualification guidance, and README only
  where it describes the affected capability;
- `cargo fmt --check`, focused tests, full locked test suite, and warnings-denied
  Clippy on Windows and Linux.

Exit gate: the independent auditor reports no untested safety claim, silent limit
increase, compatibility ambiguity, or documentation claim stronger than observed
behavior.

### Chunk 7 — Repeat the real emCP qualification

Primary support: `gpt-5.6-terra:medium`

Final evidence review: `gpt-6-astra:high`

Human authority: the operator remains the only source of permission,
accept/reject, and terminal-usability judgment.

Procedure:

1. Preserve `r4` as a failed v1 qualification record.
2. Confirm emCP is clean at the intended immutable commit.
3. Capture or validate the currently running worker profile without changing the
   worker.
4. Create a new task state and a narrowly declared verification plan.
5. Repeat the original objective and constraints.
6. Confirm the model reads only permitted bounded context and proposes a compact
   hunk for `AGENTS.md`.
7. Inspect the actual diff before re-admission.
8. Run the complete saved emCP verification check, review the offer, and make the
   explicit human accept/reject decision.
9. Complete Windows Terminal and native Linux manual-usability records separately.

Pass criteria:

- the patch response completes without approaching the 64 KiB artifact cap;
- only the intended `AGENTS.md` span changes;
- the exact old text, new text, preimage hash, postimage hash, permission, action,
  verification receipts, offer, and decision remain inspectable;
- no endpoint is started or reconfigured by Shuttle;
- no commit or worktree is created by Shuttle;
- the full emCP check passes and the human reviewer accepts the change on its
  merits. **Amended 2026-09-25 (operator):** the full emCP check passes and
  the human reviewer makes an explicit accept or reject decision on the merits
  through the task offer.

## Recommended task boundaries for Claude Cowork

Use one task/thread per chunk. Do not ask a coding task to rediscover the whole
repository. Seed it with the listed context pack, this plan, the immediately prior
contract/diff, and an explicit instruction to preserve unrelated working-tree
changes.

For Claude Cowork, use the Step 0 reassessment as the source of truth for model and
effort. Give each task the listed context pack, the immediately prior contract or
diff, the current working-tree status, and an explicit instruction to preserve
unrelated changes. Do not ask a coding task to rediscover the whole repository.

Escalate effort only when a chunk exposes an unresolved safety contradiction,
cross-platform filesystem ambiguity, or replay state that focused tests and review
cannot settle. Record the reason for escalation in the replacement model:effort
table.

## Final review questions

- Is every accepted patch deterministic from the admitted preimage and the saved
  v2 response?
- Can any model-controlled field escape the path grant or cause fuzzy matching?
- Can any current-input change occur after validation but before the first write
  without being detected at the last available boundary?
- Are all files and hunks validated before the action becomes started?
- Does every crash point produce either no effect, a completed exact postimage, or
  an inspectable unknown state with no automatic replay?
- Can old whole-file journals still be inspected without being executable under
  the new protocol?
- Is response size proportional to changed text in both the wire body and durable
  artifact?
- Can a model reach relevant text outside initial previews only through bounded,
  permitted, durable reads?
- Does the operator see the exact patch and verification evidence before accepting?
- Did the real emCP task succeed without weakening any existing gate?
