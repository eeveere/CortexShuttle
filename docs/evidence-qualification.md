# Factual producer qualification and consolidation

2026-09-08. Shuttle derives bounded native evidence from completed verification
receipts. Qualification executes no new command. The existing process executor,
source snapshots, exact verification plan and durable raw process artifact remain
the source of the observation.

## Producer boundary

`Journal::qualify_rustc` and `Journal::qualify_unittest` require an observed receipt,
unchanged pre/post snapshots, no current stale reason, normal process exit,
completed descendant cleanup and untruncated stdout/stderr. Unknown completion,
changed inputs and incomplete capture cannot become qualified evidence. This
includes source, tests, configuration, manifests and lockfiles where declared.
Qualification is charged through the existing active-time ledger.

The immutable qualification binds action ID, exact plan revision, both snapshot
IDs, process artifact hash, producer version, normalized payload and native request
key. The original receipt supplies exact executable/hash, arguments, environment,
working directory and harness runtime identity. Qualification and native outbox
intent commit in one transaction; rollback leaves neither. Native delivery uses
the shared dispatcher, and lost acknowledgement never reruns the process.

Repeating qualification returns the original historical interpretation. It does not
make a stale verification current again. The CLI includes the refreshed verification
view alongside historical evidence. Qualified Events join acceptance's episode
membership immediately after their original action Event. Both count toward the
100-Event limit, including a reserved acceptance Event.

## Exact profiles

The profiles require an explicitly selected absolute executable in the verification
plan and an empty working-directory suffix (workspace root). No PATH lookup occurs
during dispatch.

`shuttle_rustc_json_v1` requires these exact rustc arguments:

```text
--error-format=json --emit=metadata --crate-type=lib --crate-name shuttle_probe source.rs -o output.rmeta
```

`source.rs` must be a declared source input. Structured stderr diagnostics become
the native Rust compiler contract for target `shuttle_probe`. Diagnostics must have
consistent exit status, bounded shape and paths limited to `source.rs`. No prose is
scraped for type identity: `E0308` without structured expected/actual types remains
unsupported by native normalization. General Cargo invocations, Cargo test output
and other compiler languages are outside this profile.

`shuttle_unittest_capture_v1` requires these exact Python arguments:

```text
-I shuttle_capture.py test_probe --workspace . --output capture/result.json --run-id ACTION_ID
```

`test_probe.py` must be a declared test-definition input; both copied capture scripts
must be declared and exactly match Shuttle's bundled bytes. The native registry
requires Python 3.14.7. Runtime identity consists of the saved executable hash and
producer-reported runtime, not loaded-memory authentication.

The wrapper reuses the pinned CortexWeave unittest callback helper. It preserves
case counts/status, checks the raw artifact's BLAKE2s digest, and binds run ID and
exit status to the process invocation. Empty or native-ineligible runs are rejected.
Shuttle adds independently sampled test/configuration/dependency input digests to
the native bundle. Source remains in the full Shuttle snapshot rather than being
mislabeled as test configuration. The upstream helper alone samples its test-file
digest after execution, so the separate harness pre/post requirement is essential.

The scripts do not bound disk growth while arbitrary tests execute. The executor
bounds retained streams and execution time, and qualification bounds the captured
bundle. Existing grants remain responsible for allowed filesystem effects. Use the
isolated fixture, declare imported source/test inputs, and keep generated capture
output outside the input manifest. Dynamic imports, environment dependencies and
change-and-restore races are not exhaustively discovered or attested.

## CLI

First run the exact profile as an approved named check with `verify`. Then use the
full saved action ID printed by that command:

```powershell
cargo run --locked -- qualify-evidence --state-dir .shuttle/qualification --action-id FULL_ACTION_ID --producer rustc
cargo run --locked -- qualify-evidence --state-dir .shuttle/qualification --action-id FULL_ACTION_ID --producer unittest
```

These are alternative producer choices for matching observations. The unittest
`--run-id` must equal that full action ID. A producer cannot replace an existing
qualification for the same action. Qualify material failures before editing their
inputs; an already-stale result cannot be newly promoted.

## Durable consolidation disposition

New acceptance finalizations now run in this order: acceptance Event, episode
membership, episode closure, consolidation disposition, task completion, session
closure. Migration 0006 adds harness-owned qualification, preview and disposition
tables. Existing committed finalization intents remain unchanged; old accepted
runs are not retroactively given a consolidation operation.

The native preview is saved before any acceptance effect. Only a proposal marked
`Automatic` by CortexWeave can reach native acceptance, using its exact fingerprint
and proposal hash. Review-required proposals and typed no-result outcomes are
recorded without inventing an Experience. The complete disposition Event is saved
before native delivery. Restart reuses that Event; a crash after native acceptance
but before saving it retries the same native acceptance identity. Task/session
finalization waits for acknowledgement of the disposition Event.

Raw action and harness acceptance Events are not relabeled as registered repair
evidence. Their presence can yield native `UnsupportedPayloadContract`, an honest
no-result disposition. This increment qualifies producer capture and disposition
handling; it does not claim an automatic Experience from the current fixture history.
Automatic and review-required policy branches use boundary fixtures in tests;
native eligibility and acceptance idempotency remain owned by the pinned substrate.

## Qualification and remaining work

Focused tests run actual rustc failure/success and Python unittest failure/success
through Shuttle, native import and lost-acknowledgement recovery. They cover
stale/unknown receipts, tests changing during capture, empty suites and rollback.
Consolidation tests cover policy branches, saved-preview recovery, preview/result
rollback and lost delivery acknowledgement. The acceptance matrix covers all six
finalization deliveries against the native service.

Build the Linux qualification image with
`docker build -f tests/Dockerfile.qualification -t shuttle-evidence-linux tests`.
Run the normal formatting, locked tests and warnings-denied Clippy checks in that
image, mounting Shuttle source read-only and keeping Cargo/target caches separate.
The additional Python image digest records the producer runtime used in qualification.
On Windows, the tests discover installed Python and rustc before constructing exact
process specifications; Python must be 3.14.7 for native eligibility.

Preview and disposition artifacts are bounded to 64 KiB. Oversized native output
pauses finalization instead of truncating evidence. If this happens after native
acceptance, the saved preview retains the exact retry identity; reducing large
proposal payloads is not implemented by the current adapter.

No inference or embedding service is required for producer qualification. This
producer increment performed no actual user acceptance or live model-directed
write; subsequent live integration is recorded below. General
repository producers, complete episode rollover, automatic repair Experience
qualification, embeddings and resource contention remain separate work.

## Live workflow integration

The [qualified live repair command](live-repair.md) uses these producer contracts
for separate baseline/final actions and immutable capture outputs. Automated full
workflow tests reach a fresh offer and reopen without extra model calls. Live Qwen
trial 07 now also meets the exact-byte fixture check with native baseline/final
evidence. Earlier failed/stalled trials remain preserved and cannot authorize
acceptance. Trial 07 has completed actual user acceptance and native finalization,
with the explicit `unsupported_payload_contract` no-result disposition. See
implementation status for the observed trials and remaining gate.
