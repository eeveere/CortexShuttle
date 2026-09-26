# Local llama.cpp adapter

2026-09-08. The adapter runs one bounded tool proposal through the existing
controller, request ledger and fixture executor. It is transport qualification for
the development fixture; the first complete live coding task remains a later gate.

## Fixed profile and durable request

`llama-profile` observes an already running, user-managed worker. It captures the
model identifier, server build, template hash, reported context capacity, sampling
defaults, slot/template capabilities and hashes of the supplied executable and GGUF.
It checks the reported model path against that GGUF. Ordinary symlink/reparse path
components are rejected using the existing process path checker. The profile fixes
thinking mode, seed, temperature, output limit and deadline. Profiles are bounded
to 8 KiB; each runtime file is bounded to 32 GiB and hashed incrementally.

Only plain HTTP numeric loopback origins are supported. Proxies, redirects and
transport retries are disabled. Every worker request uses `autoload=false`. Shuttle
does not start, load, unload or reconfigure the service. Profile capture is a
read-only metadata operation outside a run, and makes no inference call.

Before dispatch, request intent version 2 durably records the complete profile,
exact serialized POST body, URLs, methods and the planned metadata/inference/metadata
sequence. The shared started marker commits before any of those exchanges. Each
attempt makes at most one inference POST and two metadata GETs, with no hidden
retry. The first serialized provider request fixes the profile for the entire run;
changing it requires a new run. Prepared recovery compares the exact serialization,
context, input and grant before making a call.

The request supplies four strict tools: `read_fixture`, `check_fixture`,
`replace_fixture` and `request_review`. It requires one tool call and disables
parallel calls. Replacement needs an exact observed input hash and an explicit
fixture-write grant. Review cannot accept or finalize a task. There is no model
shell or arbitrary-path tool. Tool proposals use the existing executor; no parallel
command path was introduced.

Protocol v5 uses an exact `utf8_bytes` array for replacement content, validated as
UTF-8 before passing it to the existing executor. It rejects legacy `contents`,
invalid bytes and oversized replacements; it never appends LF or reinterprets
literal escape characters. See decision S015 and [live repair](live-repair.md).

## Bounded stream and result

This section describes the streamed transport used by the fixture and admitted-planning
protocols. The admitted-editing v2 protocol below is non-streamed.

The client consumes streamed response bytes into a bounded artifact, then validates
the complete SSE stream before exposing a proposal. It requires matching model
identity, a stable response identity and tool-call identity, one choice/call,
`tool_calls` completion and `[DONE]`. Extra tool arguments, malformed JSON,
incomplete output and unknown tools fail without preparing an action. Prose and
reasoning are retained as raw data and never interpreted as commands.

Each attempted exchange records its URL, method, observed HTTP status, completion,
truncation and raw artifact hash. At most three 64 KiB artifacts commit atomically
with the immutable request result. Output insertion or result-update failure rolls
back both; reopening the started request then blocks replay as unknown. A saved
successful response can be reused after restart without contacting the worker.
Current input and grant checks still apply to its proposal. This is reuse of a
historical response, not a fresh assertion about the running service.

Reported prompt/completion usage is optional, validated when parsed and kept in
the provider observation even if later identity validation fails. A parser failure
can stop before a later usage frame; raw bounded bytes remain available. Missing
usage stays unknown. `run-status` includes provider observations for failed as well
as successful requests. Timeout records an observed failure; abrupt interruption
before a durable result becomes unknown and blocks automatic replay. Closing the
client does not prove that remote computation stopped.

The exact POST body is limited to 24,000 bytes and output to 2,048 tokens (default
512). Context includes at most eight recent action observations, each limited to
1,024 Unicode characters with a truncation marker. A byte-based context estimate
adds output allowance and a 4,096-token reserve before comparing reported capacity.
This is a conservative admission estimate, not exact tokenizer/template accounting.
The run's existing attempt, active-time and stall limits remain in force.

## CLI

Use absolute paths to ordinary executable and model files, and an existing output
directory. The profile command creates a new file and refuses to overwrite one.

```powershell
cargo run --locked -- llama-profile --model "MODEL_ID" --server-executable "C:/path/llama.exe" --model-file "C:/path/model.gguf" --output .shuttle/worker.json
cargo run --locked -- llama-step --profile .shuttle/worker.json --state-dir .shuttle/model-run
cargo run --locked -- run-status --state-dir .shuttle/model-run
```

Each `llama-step` advances at most one provider/action step. Writes are disabled
unless `--approve-fixture-writes` is supplied. `--thinking` at profile capture makes
a separate fixed thinking-enabled profile; use a separate run when switching.
The terminal preview remains illustrative; transport streaming is not yet connected
to live token display.

## Live observations

The existing Windows worker was observed on 2026-09-08 at port 8080, using
`unsloth/Qwen3.5-4B-GGUF:Q4_K_M`, build `b10217-ddd4ec142`, reported context 175,104
and four slots. Seed 42, temperature 0 and output limit 512 were fixed per profile.

| Thinking | Independent read steps | Observed elapsed milliseconds | Output tokens |
| --- | --- | --- | --- |
| Disabled | 3 succeeded | 5509, 5556, 5548 | 14, 14, 14 |
| Enabled | 3 succeeded | 6770, 7658, 6982 | 83, 114, 83 |

The disabled run was reopened for a second step: Qwen requested `check_fixture`,
with 927 input/15 output tokens and 5759 ms observed elapsed time. The check
correctly reported the unchanged `41` fixture as failing. All six fixtures remained
unchanged, and none of these calls had a write grant. Local evidence is retained
under ignored `.shuttle/llama-live/` as profiles, journals and CLI result logs.

Captured BLAKE3 identities:

- Executable: `d0334f1f8d91e4df4bdac9c7aff40d4a3a4d46580005979aa859c700a6e66f49`
- GGUF: `e1f86109e15fc9f1aef5b9d62dae801bdb9d2fa73f4d7b1c70c4257984132bfe`
- Template: `09c702c139a8905b1b3dfc3d5d3a32b593b853be72ae8428e2fac3137278a140`

These observations establish valid read/check tool transport and recorded usage for
the sampled profiles. They do not establish repair quality, deterministic reasoning,
GPU residency, maximum context performance or embedding behavior. Port 8081 was not
needed or used. Real factual producers and a supported consolidation disposition
must be qualified before milestone 0b's first live model-directed write.

## Admitted-task planning

`task-plan --state-dir <task> --profile <worker.json>` uses a separate
`shuttle-llama-admitted-planning-v1` protocol for one admitted general task. It
captures a bounded preview of only the admission's declared files, together with
the immutable objective, constraints, snapshot identity, check names and waivers.
The saved source snapshot and file hashes remain authoritative; previews may be
truncated or unavailable for non-UTF-8 files.

The protocol exposes one tool, `record_task_plan`. Its bounded summary, proposed
workspace-relative paths and limitations are stored as data through the existing
durable model-request ledger. It has no file, shell, process, permission,
acceptance or finalization tool. A request intent is saved before the worker call;
a started request becomes unknown on restart and blocks replay. A saved successful
response is applied atomically with its proposal and is reused without a second
worker call. If inputs change while the worker runs, the response is discarded and
the run pauses. Re-admission retires the old context and proposal permanently.

This is phase 1a context assembly and planning only. It does not authorize the
proposed paths for a write or execute verification. Those controls belong to the
following explicit write-permission phase.

## Admitted-task write-permission boundary

`task-write-status --state-dir <task>` presents the current saved proposal,
normalized allowed paths, permission actor/timestamp and any stale or revocation
reason without migrating or changing journal state. After review, a caller may use
`task-write-grant` with the exact `context_id`, an idempotency key and explicit
`--approve-proposed-paths true`. The command does not contact the model or run a
tool; it stores only a human permission record. `task-write-revoke` irreversibly
withdraws that record.

`task-edit-review --state-dir <task>` shows what an admitted v2 edit changed, or
why it was blocked, from saved state alone: the session ledger (turns, reads,
refusals, rejected turns), then the edit's file and hunk counts, pre/post
hashes and sizes, the exact compact hunks, and whether each file's read-back
hash matched. Prepared, cancelled and unknown actions say plainly what did and
did not happen. Saved text is escaped: every control character and a fixed list
of bidirectional, zero-width, tag and other invisible characters is shown as a
`\u{..}` escape (the exact list is in S033's Chunk 5 clarification, item 4). The
`--json` output and the offer review's JSON pass through the same escaping. The same lines appear in the terminal task
view and, with the full hunks, in `task-offer-review`. Historical whole-file
actions render as file summaries only. Some tasks hold only a historical v1
edit request that failed before any action was prepared, like the retained r4
run. Such a task has no review, and `task-edit-review` says a v1 request is
recorded rather than that no edit was attempted. With `--json` it prints
`null`, which means only that the task has no v2 session and no edit action.

The record is bound to one proposal request and snapshot. Paths must be nonempty,
UTF-8, workspace-relative normal components; absolute, parent-traversing and
duplicate paths are rejected. There is no time-based expiry, but re-admission
stales old permission and a current declared-input mismatch makes it unusable.
The grant is not itself an edit: the edit protocol below rejects links and reparse
points and revalidates these bindings before it prepares an action and again
immediately before its first write.

## Admitted-task patch protocol

`task-edit --state-dir <task> --profile <worker.json>` uses
`shuttle-llama-admitted-editing-v2` (decision S033) only after a current write
permission exists. The provider identity is
`shuttle-llama-admitted-editing-v2:<profile_digest>`, and the adapter is bound to
one durable edit session. The same worker profile as planning is used; the saved
request binds the specific protocol. The request is non-streamed
(`stream:false`) and the response must be identity-encoded `application/json`
within the unchanged 64 KiB per-artifact cap.

The model gets at most five turns, each a durable, started-before-POST request
with exactly one tool call:

- `read_task_text {path, start_line, line_count}`: up to 128 lines of a granted,
  declared UTF-8 file;
- `find_task_text {path, literal}`: exact literal search, at most eight matches;
- `record_task_patch {files: [{path, expected_file_hash, hunks: [{old_utf8,
  new_utf8}]}]}`: the compact patch.

At most four reads succeed, with 2,048 excerpt bytes each and 6,144 in total.
Read tools are offered only while a read can still succeed, and the fifth turn
offers only the patch. Observations are saved and replayed byte for byte, never
reread. A read whose next request would not fit is shortened, or refused with the
session kept open for a patch.

A hunk's `old_utf8` must occur exactly once in the admitted preimage (overlapping
matches count), and all hunks resolve against that same preimage with no fuzzy
matching. A patch is limited to 8 files, 16 hunks per file (64 total) and 4,096
bytes of old plus new text, so the reply is proportional to the changed text,
not the file. The decoder parses the response as typed structures, rejects
duplicate and unknown fields in every argument object, and never unescapes text a
second time.

The patch reply and its prepared `PatchWorkspaceFiles` action commit together
before any write. Application re-checks permission, the full declared snapshot
and every target's path and handle, commits the started marker, then writes each
target in place through its validated handle, reads it back, and records success
only if the post-write snapshot equals the predicted one. There is one
exception, on Windows only: the ARCHIVE and NORMAL attributes of the patched
files are left out of that comparison, because NTFS sets the archive bit on an
in-place write (S033's Chunks 4b-4d clarification, item 4). Every other
attribute and every unpatched file must match exactly.

If one of application's own checks fails before the started marker, the
action is cancelled, and a cancelled action is never resumed. The run-level
guards inside the journal's `start` are the exception: acceptance seal,
execution budget or stall, a pending model request, or a run that is not
ready. Failing one of them leaves the action `prepared`, and it resumes once
the condition clears (same clarification, item 2). Any failure after the
started marker is `unknown`. A started write is never retried, rolled back or
replayed, and a cancelled or unknown outcome needs a fresh task state. This is
journal-level transactional preparation, not a multi-file
filesystem transaction: a concurrent writer can race the checks, and the
comparison is a boundary observation rather than an atomic snapshot.

A session that fails any turn (transport, decoder or domain rejection, drift,
exhausted budget) closes, pauses the run and keeps its evidence. It is not
reopened; continuing needs a fresh task state with a new admission and grant. A
run that holds any saved v1 whole-file edit request or action cannot start a v2
session. Nothing generates v1 edit requests any more, and historical whole-file
actions remain readable, offerable and reviewable as file summaries. The v2
protocol was qualified once against the live worker on the real emCP task
(2026-09-25, [record](manual-qualifications/emcp-2026-09-25-chunk7.md)): the
patch turn completed compactly and applied exactly, and the operator rejected
the patch on its merits. That qualifies the protocol, not task success; see
S033's closure clarification.

After a successful patch, declared inputs no longer match the old admission. Run
the explicit `task-readmit`, then `task-verify-all` and `task-verify-evidence` to
create fresh suite evidence. No patch or passing check accepts or finalizes work.

## Limits and protocol reference

The qualified live workflow now uses `shuttle-llama-fixture-v5`: compact factual
history plus reconstructed assistant/tool pairs for completed fixture actions.
Earlier live observations above used v1. Protocol changes alter provider identity
and cannot be substituted into an existing run. See [live repair](live-repair.md).

Executable/GGUF hashes attest local files, not loaded memory or the association of
a listening process with the supplied executable. Metadata and file identity are
sampled before and after successful inference, without an atomic server/file lock;
change-and-restore races are possible. Backend libraries, GPU placement and KV-cache
residency are unqualified. File hashing is synchronous and contributes to observed
active time; cooperative deadlines cannot preempt it. Error paths may have only a
partial exchange sequence. Remote/authenticated endpoints and production repository
context assembly remain outside this increment.

The routing and SSE contracts follow the upstream
[llama.cpp server documentation](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md).
The live build identity above, rather than the moving documentation branch, records
the actual worker used for these observations.
