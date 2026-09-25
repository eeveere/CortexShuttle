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
actions render as file summaries only.

The record is bound to one proposal request and snapshot. Paths must be nonempty,
UTF-8, workspace-relative normal components; absolute, parent-traversing and
duplicate paths are rejected. There is no time-based expiry, but re-admission
stales old permission and a current declared-input mismatch makes it unusable.
This still is not an edit protocol: a later phase must reject links/reparse points
and revalidate these bindings immediately before writing durable edit intent.

## Admitted-task patch protocol

`task-edit --state-dir <task> --profile <worker.json>` uses
`shuttle-llama-admitted-editing-v1` only after a current write permission exists.
Its sole tool returns one bounded list of `{path, expected_hash, utf8_bytes}`
replacements. Paths must be within the granted set and identify existing declared
regular files; additions and deletions are not supported. Shuttle stores the model
reply and the prepared action atomically, marks the action started before writing,
and never replays an unknown completion. The same local worker profile identity is
used for planning and patching; the persisted serialized request still binds the
specific protocol.

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
