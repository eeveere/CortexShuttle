# Qualified isolated live repair

`live-repair` connects the existing model controller, process executor, producer
qualification and acceptance offer. It creates a dedicated fixture whose only
model-editable file is `value.txt`, initially `41` followed by LF. The fixed Python
unittest asserts exact bytes `42` followed by LF. This is a protocol qualification,
not a repository coding benchmark.

```powershell
shuttle live-repair --profile .shuttle/live-repair-worker.json `
  --state-dir .shuttle/my-live-repair --python C:/Python314/python.exe `
  --approve-fixture-writes --approve-host-execution
```

The caller provides the running local worker and captured profile. Python must
match the native producer eligibility registry (currently 3.14.7). No embedding
service is involved. Both execution grants are explicit CLI requirements.

## Durable stages

Before work, save the provider identity and exact baseline/final plans in a
versioned configuration. Restart compares the complete configuration; it cannot
repurpose a prior run with another provider, Python binary or environment. Both
plans declare source, manifest, test and capture scripts, with no waivers. Each
action has a separate capture output path and exact plan revision. Generated
capture outputs are outside the declared input manifest.

The baseline uses the existing process executor, pre/post snapshots, immutable
receipt/result/outbox transaction and native unittest qualification. Only the
expected single assertion failure permits the baseline-to-model transition.
That transition retains consumed budgets, actions and evidence. An unknown
baseline or model request blocks replay. An unexpected passing baseline cannot
authorize model writes.

The model uses the existing read/check/replace/review tools. Fixed test and runner
bytes are validated on open and immediately before a fixture action; validation
reads are bounded. A changed definition is rejected before an ordinary fixture
write. Review requests trigger a second real process verification and native
qualification. Only its fresh passing receipt can support an acceptance offer.
The failed baseline remains historical evidence and becomes stale after the edit.

Reopening an offered run returns the same offer through the normal freshness
check without another model request. The CLI prints the review; it does not accept
on the user's behalf or finalize the task. Actual user acceptance and subsequent
native disposition are separate gates.

## Provider presentation and recovery

Adapter protocol `shuttle-llama-fixture-v5` presents saved actions in order using
tool names, states, before/after input hashes and explicit `check_passed`. Harness
processes are labelled separately; their large specifications and numeric byte
reports are omitted from the provider presentation. Fixture observations remain
data. Completed fixture actions are also reconstructed as paired assistant tool
calls and tool-result messages with matching deterministic IDs. These messages
come only from saved action results, not inferred success or a new execution.
The complete original context, exact serialized wire request and transport
artifacts remain in the journal. Protocol identity changes prevent silent reuse
of earlier requests. No automatic model retry or stall-budget reset is added.

Replacement arguments use `utf8_bytes`: integers 0–255 representing the exact
UTF-8 file content. LF is byte 10; no newline is inferred or appended and no escape
sequence is expanded. Invalid UTF-8, non-byte values and decoded content above
16,384 bytes are rejected. The legacy string argument is rejected under v5.
Saved replacement calls are reconstructed with the same bytes. Read results also
expose bytes of the saved bounded observation, with its truncation limitation.
The 24 KiB request and 64 KiB response bounds still apply, so byte-array encoding
can reduce the practical content size below the executor's maximum.

The server-facing schema intentionally omits `maxItems`: this worker expands a
large array limit into a grammar repetition beyond its supported limit. Shuttle's
decoder still enforces the full replacement size bound before preparing any
action. Trial 06 (v4) records the rejected schema and HTTP 400 with no retry;
v5 changes the provider identity and runs in a separate journal.

The live executor has a stable native session mode across its process and fixture
stages. Nested futures are boxed at the workflow boundaries so the actual Windows
CLI runs within its main-thread stack. A mock HTTP CLI regression covers the whole
repair and a second invocation with no extra inference calls; it also verifies
failed/passing checks in the compact provider history.

## Limits

Snapshots and path checks detect ordinary changes; they do not provide atomic
isolation from a hostile concurrent writer. The tiny test covers only exact fixture
content. The model cannot edit the fixed tests or use general host commands.
Runtime identity does not attest loaded libraries or GPU memory. Native
consolidation may honestly return `UnsupportedPayloadContract` for raw fixture
history. Neither a passing test nor an acceptance offer claims an automatic repair
Experience. Platform and live outcomes are recorded in
[implementation status](implementation-status.md).
