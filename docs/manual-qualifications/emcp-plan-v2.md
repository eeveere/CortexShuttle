# emCP verification plan v2 (K4)

2026-09-26. The verification plan for the next emCP live run (K6). It replaces the
known-broken `.shuttle/emcp-agents-live-test-plan.json` (`npm.cmd`, a
`SystemRoot`-only environment) and plan `e` from the
[Chunk 7 record](emcp-2026-09-25-chunk7.md), which is stale on this machine. This
record is docs only. The plan and the check script live in the ignored `.shuttle/`
directory, so this file holds what is needed to reproduce them.

## What changed from plan `e`

- **Paths.** Plan `e` names `C:\dev\agentic\emCP` for the workspace root and for
  `--prefix`. That directory no longer exists. emCP is at `C:\den\agentic\emCP`.
  The intake is an immutable singleton, so the plan needs a new state directory in
  any case.
- **`--prefix` removed.** Since K3b the child's working directory is passed without
  the verbatim `\\?\` prefix, so the workaround is unnecessary. The baseline run
  below confirms it.
- **Inputs widened from two files to four** (see the arithmetic below).
- **A second check, `doc-scripts-exist`.**
- **Unchanged.** `node.exe` v24.19.0 (hash `38b9aed0…`, verified today with
  `shuttle process-spec`) launching `npm-cli.js`, the complete explicit
  environment (`ComSpec`, `PATH`, `SystemRoot`, and `TEMP`/`TMP` outside the
  workspace), and the 300,000 ms limit.

## Declared inputs and why these four

`npm run check` is `typecheck && lint && format:check && test`. It reads about 103
files (60 source, 26 test, docs and configs), and `prettier --check .` covers
`AGENTS.md`. The plan deliberately declares only four:

| Input | Kind | Bytes | Preview |
| --- | --- | --- | --- |
| `AGENTS.md` | source | 11,643 | 1,024 (the permitted target) |
| `package.json` | dependency manifest | 1,395 | 1,024 (the scripts) |
| `tests/e2e/mcp-stdio.test.ts` | test definition | 7,545 | 1,024 (imports show the fake embedding server) |
| `vitest.config.ts` | runner configuration | 379 | 379 (the live test is excluded unless `RUN_LIVE_EMBEDDING_TEST=1`) |

Two budgets bound the choice, and both were measured:

- **Preview pool (S034).** Planning gives every declared file at most 1,024
  preview bytes and the whole context 4,096, allocated in path-sorted order.
  Confirmed from the saved snapshot: the four files above receive 3,451 bytes, all
  non-empty, with 645 to spare. Declaring `tests/integration/live-embedding.test.ts`
  as well would use the pool up before `vitest.config.ts`, which sorts last, gets a
  byte.
- **Edit-session context.** Each declared file costs about 184 bytes of the
  24,000-byte durable context. Declaring all 103 files the check reads would spend
  about 19 KB of it and starve the reads.

Left out on purpose: `package-lock.json` (the check does not read it, and prettier
ignores it), `tests/integration/live-embedding.test.ts` (`package.json` and
`vitest.config.ts` already state the fact), and everything under `src/`.

**Consequence.** Receipts go stale only when a declared file changes. An edit to
undeclared source or tests does not stale them. That is the "declared inputs only"
limit in the verification contract, and it is acceptable for a documentation-only
objective. Do not reuse this plan for a task that edits source.

## The `doc-scripts-exist` check

A read-only Node script that reads `package.json`, then scans `AGENTS.md`,
`README.md` and `docs/*.md` for `npm run <name>` and `npm run-script <name>`, and
fails if a named script is not in `package.json`. Exit 0 means all exist, 1 lists
the missing references, 2 means the check itself could not run. It writes nothing.

- It would have caught attempt 7g's invented `npm run test:mcp`, and it makes K6's
  criterion "references only existing scripts" machine-checkable. Evidence, not a
  filter: Shuttle adds no semantic check of command names.
- Tested on a scratch copy of emCP's documents (nothing in emCP was touched): the
  baseline passes with 28 references in 10 files; appending
  ``Run `npm run test:mcp` ...`` to a copy of `AGENTS.md` exits 1 and names
  `AGENTS.md:356`; an empty `package.json` exits 2.
- **Limit.** The script lives outside the workspace, so the source snapshot does
  not bind its bytes, and a declared input must be under the workspace. Its
  SHA-256 for reference is
  `e42ef0d38a49a27445b02096c6a905d4e2ac2bb1daea77f69f9bc44dde5d7229`
  (`.shuttle/emcp-doc-scripts-check.mjs`, 2,088 bytes). A changed script changes the
  check's meaning without staling any receipt.

## Validation done (no emCP file touched)

- `shuttle task-intake` and `shuttle task-preflight` in a scratch state directory
  outside the repository. Plan revision
  `20f0565b0588fb44a3aed10297c0a3279639bc0f673be6d54528e40e6cd15928`; both saved.
  Preflight ran the K3a launch checks on both checks. No process was dispatched and
  no model was called.
- `git status --porcelain` in emCP was empty before and after (HEAD `a652789`).
- The model-free baseline run executes `npm run check` in emCP, so the operator ran
  it afterwards; see "Baseline outcome" below.

## Operator baseline run (model-free)

Use a state directory that K6 will not reuse; state cannot be reset.

```powershell
git -C C:\den\agentic\emCP status --porcelain     # expect no output
$sh = ".\target\debug\shuttle.exe"; $state = ".shuttle\k4-baseline"
$obj = "Correct AGENTS.md so its testing guidance accurately distinguishes the opt-in live embedding test from the MCP stdio end-to-end test."
$con = "Modify AGENTS.md only; preserve existing npm commands and behavior; do not change TypeScript source, tests, package/lock files, runtime/model-server configuration, or generated artifacts; do not start or reconfigure an endpoint; keep the live test explicitly gated by RUN_LIVE_EMBEDDING_TEST=1"
& $sh task-intake --state-dir $state --workspace C:\den\agentic\emCP --objective $obj --constraint $con --plan .shuttle\emcp-agents-live-test-plan-v2.json
& $sh task-preflight --state-dir $state
& $sh task-admit --state-dir $state
& $sh task-verify-all --state-dir $state --approve-host-execution
& $sh task-verify-status --state-dir $state
git -C C:\den\agentic\emCP status --porcelain     # expect no output
```

Expected, from attempt 7g and the script test: `full-check` passes (about 26 s; 19
test files, 125 tests) and `doc-scripts-exist` passes (28 references). If
`full-check` reports `cmd.exe` "UNC paths are not supported", the K3b working
directory change did not take effect; if it writes an `undefined/` directory, the
environment lost `TEMP`/`TMP`. Either is a finding to record, not a reason to edit
the plan silently. If the `node.exe` hash no longer matches after a Node upgrade,
regenerate the specifications with `shuttle process-spec` and expect a new plan
revision.

### Before K6's intake

K6 needs its own new state directory. Do not reuse `.shuttle\k4-baseline`; an
intake is immutable and a state directory cannot be reset.

K6 must also use the exact objective and constraint text of attempt 7g, which is
what `$obj` and `$con` above contain. Paste them from this record; do not retype
them. The baseline run below was affected by exactly that slip. Before
`task-intake`, compare both strings with 7g's by SHA-256 of their UTF-8 text (no
trailing newline):

```powershell
function Get-Sha256($s) { [BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash([Text.Encoding]::UTF8.GetBytes($s))).Replace('-','').ToLower() }
Get-Sha256 $obj   # expect b224840975b69884e00326a6f1e3d5f44bb01a4a837bc5df9553ee2014987475
Get-Sha256 $con   # expect 78a598b04607f62503ede7e3edfbfa06021c3fa8e4255c0400bb7ee71b27aa9b
```

Those two values are the hashes of 7g's saved objective and constraint, read from
its journal. The snippet was run against the text above and reproduces them; a
constraint with `agents.MD` in place of `AGENTS.md` hashes to `d56c6258…` and does
not match. If either value differs, stop and fix the string before the intake.

## Baseline outcome (operator run, 2026-09-26)

The operator ran the runbook above in `.shuttle\k4-baseline`, with no model and
`--approve-host-execution` for the two checks. Result: **passed.**

- **Tree state.** `git -C C:\den\agentic\emCP status --porcelain` was empty before
  and after the run.
- **Identities.** Intake `03045198-3257-4c64-a74a-ceeba8d664a2`, admission
  `4f04a764-3707-4188-8fbe-b9d4fe45124a`, run
  `6d66e622-2930-414b-8b1c-aae8f48eb145`. The plan revision is
  `20f0565b0588fb44a3aed10297c0a3279639bc0f673be6d54528e40e6cd15928`, and the
  preflight snapshot is
  `d1b19495e5a7d61ed1b64025a32a413a4e336a72c92a306e9b253b7994a45824`. Both are
  identical to the scratch validation above.
- **Verification.** `task-intake`, `task-preflight` and `task-admit` succeeded, and
  `task-verify-all` reported `full-check` and `doc-scripts-exist` both succeeded.
  `task-verify-status` shows both `passed`, with no stale reason, equal pre- and
  post-snapshots and no waivers. Suite evidence is null, as expected: the run did
  not include `task-verify-evidence`. A read-only look at the journal
  (`mode=ro&immutable=1`) agrees: two `verification_receipts` with no stale reason,
  and two actions in state `succeeded`.
- **K3b confirmed live.** `full-check` ran with no `--prefix` and `cwd ""` and
  passed, so the verbatim working-directory fix works against the real emCP check.
- **Not confirmed.** The "19 test files, 125 tests" and "28 references" figures and
  the run duration are not in the receipts (stdout is not kept there), so this run
  does not confirm them. Only the pass status is established.

**Deviation.** The operator typed the constraint by hand and wrote
"Modify agents.MD only" instead of "Modify AGENTS.md only". Intakes are immutable,
so `.shuttle\k4-baseline` permanently holds the lowercase wording; the journal
confirms it, and the objective there does equal 7g's. It does not affect this
run: no model was called, the constraint is not part of the plan revision (the
revision matches the scratch validation, whose text was correct), and the state
directory is spent anyway. It is recorded so nobody mistakes that intake's
constraint for 7g's, and it is the reason for the K6 comparison above. The
runbook's text was already exact; it equals 7g's saved objective and constraint
character for character.

## Plan file

`.shuttle/emcp-agents-live-test-plan-v2.json`, SHA-256
`82cee53a283fb7b7af8c0837687bc08c7a96b812a54620d82ef8c8ca7ccc87f9`. It was written
from `shuttle process-spec` output so no hash or path is typed by hand. Its checks:

| Check | Executable | Arguments | Limit | Environment |
| --- | --- | --- | --- | --- |
| `full-check` | `\\?\C:\Program Files\nodejs\node.exe` (`38b9aed0…`) | `C:\Program Files\nodejs\node_modules\npm\bin\npm-cli.js run check` | 300,000 ms | `ComSpec`, `PATH` (`C:\Program Files\nodejs;C:\WINDOWS\system32`), `SystemRoot`, `TEMP`, `TMP` (`C:\Users\Capta\AppData\Local\Temp`) |
| `doc-scripts-exist` | the same `node.exe` | `C:\den\CortexShuttle\.shuttle\emcp-doc-scripts-check.mjs` | 30,000 ms | `SystemRoot` |

Both use `cwd ""` (the workspace root), 4,096 output bytes per stream and no idle
timeout. Inputs are the four above, with no exclusions or waivers.
