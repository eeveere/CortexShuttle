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

## K6 runbook (live run, operator only)

K6 is the same supervised workflow as attempt 7g, run once more against emCP with
plan v2 and the S034 code (context revision 2). It answers one question: given
enough context, does the 4B worker produce a correct patch for the original
objective? Every live step below is yours; nothing here should be run by an
assistant. It is one sample, and one probe found that an identical request can
produce different output on this worker, so do not read a single result as a rate.

### Before you start

- [ ] **Build with S034.** `cargo build --locked` from a checkout at commit
  `d843efa` or later (revision-2 sessions), and use that `target\debug\shuttle.exe`.
- [ ] **emCP is clean.** `git -C C:\den\agentic\emCP status --porcelain` prints
  nothing. Note `git -C C:\den\agentic\emCP log -1 --format=%H`.
- [ ] **The worker is the 7g worker.** `curl.exe -s http://127.0.0.1:8080/props`
  reports build `b10278-d52ec04a6` and model
  `C:\models\workers\Qwen3.5-4B-Q4_K_M.gguf`. Do not start, stop or reconfigure it
  during the run.
- [ ] **Use 7g's saved profile** (`.shuttle\chunk-7g\emcp-worker-profile.json`:
  thinking off, 512 tokens, seed 42, temperature 0). Its digest is `c02198cf…`. If
  you capture a fresh one with `llama-profile`, its digest must match; a different
  digest is a different experiment.
- [ ] **A new state directory** that has never been used, for example
  `.shuttle\k6-<date>`. Not `k4-baseline`, and not any `chunk-7*` directory.
- [ ] **The exact 7g objective and constraint.** Paste `$obj` and `$con` from the
  baseline runbook above, and check both hashes (see "Before K6's intake") before
  `task-intake`.

### The run

```powershell
$sh = ".\target\debug\shuttle.exe"; $state = ".shuttle\k6-<date>"
$profile = ".shuttle\chunk-7g\emcp-worker-profile.json"
# $obj and $con pasted from the baseline runbook; hashes checked.

# 1. Intake, preflight, admission (no model, no commands)
& $sh task-intake --state-dir $state --workspace C:\den\agentic\emCP --objective $obj --constraint $con --plan .shuttle\emcp-agents-live-test-plan-v2.json
& $sh task-preflight --state-dir $state
& $sh task-admit --state-dir $state          # note the admission id it prints

# 2. Read-only plan (one model request)
& $sh task-plan --state-dir $state --profile $profile
& $sh task-write-status --state-dir $state   # read the proposal; note the context id

# 3. Grant: only if the proposal names exactly AGENTS.md
& $sh task-write-grant --state-dir $state --context-id <context id> --request-key k6-grant-1 --actor <you> --approve-proposed-paths

# 4. The edit (one shot, no retry), then inspect it
& $sh task-edit --state-dir $state --profile $profile
& $sh task-edit-review --state-dir $state
git -C C:\den\agentic\emCP diff

# 5. Re-admission, then verification (both checks)
& $sh task-readmit --state-dir $state --from-admission <admission id> --request-key k6-readmit-1 --reason "AGENTS.md changed by the K6 patch"
& $sh task-verify-all --state-dir $state --approve-host-execution
& $sh task-verify-status --state-dir $state
& $sh task-verify-evidence --state-dir $state

# 6. Offer and your decision
& $sh task-offer --state-dir $state --request-key k6-offer-1
& $sh task-offer-review --state-dir $state
& $sh task-offer-respond --state-dir $state --offer-id <offer id> --choice <accept|reject>
& $sh task-finalize --state-dir $state       # only after an accept, if delivery needs resuming
git -C C:\den\agentic\emCP status --porcelain
```

The TUI (`shuttle task --state-dir $state ...` with the double-confirmed shortcuts in
Part B of the manual guide) runs the same durable steps; use whichever you used for
7g. `task-admission-history --state-dir $state` prints the admission ids again.

Stop and record instead of trying another command when any of these happens:

- the proposal names a path other than `AGENTS.md` (reject; do not grant);
- a request is `prepared`, `started` or `unknown`, `task-plan` or `task-edit` fails
  or runs to the token limit, or the edit session closes without a patch. A closed
  session cannot be reopened, so continuing needs a new state directory, and the
  spent one stays as evidence;
- the emCP diff is more than `AGENTS.md`, or `git status` shows anything after
  verification that was not there before.

### After the edit: check the transcript

Right after `task-edit` (before re-admission) dump what the model saw and did. The
script `docs/manual-qualifications/k6-transcript-dump.py` opens the journal through
SQLite's read-only, immutable URI, so it cannot change the run:

```powershell
python docs\manual-qualifications\k6-transcript-dump.py --state-dir $state `
  --span AGENTS.md:238-242 --span AGENTS.md:346-350 `
  --expect 'test:live' --expect '"test": "vitest run"' --expect "RUN_LIVE_EMBEDDING_TEST === '1'"
```

It prints the session (context revision, the reference files shown, omissions, the
plan-summary state), each turn with the lines and bytes it read, the patch, and
informational checkboxes for the criteria below. It reaches no verdict. The two
`--span` ranges are the lines the objective targets, confirmed against the current
`AGENTS.md` (355 lines, 11,643 bytes): lines 238-242 ("End-to-End Tests", which
wrongly says the e2e tests run against a live endpoint with `npm run test:live`) and
lines 346-350 ("Unit tests only" and "Integration tests only", which mislabel the
two commands). The `--expect` literals come from `package.json` and
`vitest.config.ts`, which should now appear as reference files. Run it against 7g
for comparison: `--state-dir .shuttle\chunk-7g` shows no reference files, neither
span read, and `test:mcp` invented.

### What counts as success, and how to read a failure

K6 succeeds when all of these hold (S034 Delivery): the edit transcript contains
the target spans and the `package.json` scripts; the patch changes those spans and
names only existing scripts; `full-check` and `doc-scripts-exist` both pass; and
you accept the patch on its merits. Read the outcome this way:

| What you see | What it means |
| --- | --- |
| The dump shows a target span or the scripts never appeared | A context or strategy failure, not a capability failure: the model did not use `find_task_text` or read far enough. Consider prompt guidance, not a bigger worker. |
| The transcript has the text but the patch is still wrong or names a missing script | Capability with sufficient context. This is the input to the larger-worker experiment in S034, not to a change here. |
| `doc-scripts-exist` fails | The patch named an `npm run` script that `package.json` does not define; the check did its job. |
| The patch is right and both checks pass | Judge it on its merits and record accept or reject. |
| The session closed on an outside-path read or patch | Record it. The enum guards only the patch tool, and the journal closes the session on any path outside the permission. |
| `task-edit` ended at the token limit with no patch | Record it. The live probe saw this on 3 of about 27 requests that carried a path enum. |

Save the dump output and the review outputs somewhere outside the state directory
(for example `.shuttle\k6-notes\`), and record: the state directory, the intake,
admission and run ids, the plan revision, the profile digest, the worker build, the
Shuttle commit, emCP's HEAD, and your decision. Then add a "K6 outcome" section to
this record, as the baseline run did.

## K6 outcome (operator run, 2026-09-28)

The operator ran the K6 runbook in `.shuttle\k6-280926` and rejected the offer.
Result: **the workflow completed; the patch did not meet the objective.** It is
one sample.

- **Setup.** The objective and constraint hashes matched 7g (`b2248409…`,
  `78a598b0…`). Profile `.shuttle\chunk-7g\emcp-worker-profile.json`, digest
  `c02198cf…`. Worker build `b10278-d52ec04a6` with
  `C:\models\workers\Qwen3.5-4B-Q4_K_M.gguf`, unchanged from 7g, with the same
  startup arguments. emCP HEAD `a6527896e6ea0fea047f7425c60e4db877c7aacb`, the same as the
  baseline. Shuttle at `7edcb0f`.
- **Identities.** Intake `a0e1b4bc-5182-4bdc-99a5-5632bf4cbbaf`, first admission
  `05de0e17-1443-435b-bf00-57c96ec09bb7`, re-admission
  `1f4f801c-9329-4130-968e-52479e10c7d6`, run
  `e9f2cde9-0ce5-4aec-a947-8b1891c9433c`. Plan revision `20f0565b…` (unchanged),
  preflight snapshot `1554dc79…`, post-edit snapshot `21b50708…`. Context
  `9c42eebb…`, permission `fe3dbe4e-4382-4efc-857b-9807de67ff37`, edit session
  `aca745fcb041`, offer `d56a2b84-b6db-4827-95e8-967846eb86cf`.
- **Plan.** One model request. The proposal named only `AGENTS.md`, so the grant
  was made. Its summary already framed the task wrongly: "the MCP stdio end-to-end
  test is the default behavior".
- **Edit.** Context revision 2, with all three reference files shown (none
  omitted) and the plan summary included (158 B). Turns 0 to 2 read `AGENTS.md`
  from the top in 2,048-byte reads (lines 1 to 169). That used 3 of 4 reads and
  all 6,144 read bytes. Turn 3 recorded the patch. There was no `find_task_text`
  call, no token-limit stop and no retry. It made five model responses in all.
- **Patch.** One hunk in the Essential Commands block (bytes 351 to 495, 11,643 to
  11,717 bytes). It added "opt-in" to the live-test comment, which was already
  correct. It also added a second `npm run test`, labelled "Run MCP stdio
  end-to-end tests (default behavior)", two lines under the existing one. Neither
  target span changed. "End-to-End Tests (tests/e2e/)" still says the e2e tests
  run against a live endpoint through `npm run test:live`. "Testing" still labels
  `npm run test` "Unit tests only" and `npm run test:live` "Integration tests
  only". The emCP diff was `AGENTS.md` only, and the read-back matched.
- **Verification.** `full-check` and `doc-scripts-exist` both passed on the
  re-admitted snapshot. Their pre- and post-snapshots were equal, with no stale
  reason and no waivers. Suite evidence was recorded. Both checks passed because
  the patch broke nothing: prettier accepts it and both named scripts exist.
  Neither check can see whether the objective was met.
- **Decision.** Rejected from the terminal UI (decision key
  `terminal/task/d56a2b84-…/reject`) because the patch leaves both wrong spans in
  place and adds a redundant command.
- **Transcript criteria.** Both target spans were **not covered** (238-242 and
  346-350). The `test:live`, `"test": "vitest run"` and
  `RUN_LIVE_EMBEDDING_TEST === '1'` literals were all seen, in reference previews
  and the turn-0 read. The dump is saved in `.shuttle\k6-notes\transcript-dump.txt`.

**Reading.** This is the first row of the outcome table: a context or strategy
failure, not a capability failure. The table misses one detail. A 6,144-byte read
budget cannot reach line 238 of an 11,643-byte file through reads from the top,
so a correct patch needed `find_task_text`. The objective gives no location, so
the model patched the only testing block it had seen, which matched the plan's
wrong framing. The next step points at the read budget or the prompt's
`find_task_text` guidance, not the larger-worker experiment.

**Findings to record.**

- **Rejecting does not revert the workspace.** After the rejection, emCP still
  showed ` M AGENTS.md`. Restoring the file is a manual step. The operator reverted
  it, and `git status --porcelain` in emCP is empty again. The run's phase is
  `ready`.
- **Checks can pass without the objective being met.** `full-check` and
  `doc-scripts-exist` show only that nothing broke. Judging the objective is left
  to the operator's decision.
- **Runbook placeholders.** `<date>`, `<context id>` and `<admission id>` caused
  three failed commands: an OS path error and two PowerShell `<` parse errors.
  None of them reached the journal. The transcript dump was not run during the
  live sequence. It was run afterwards against the closed journal, which gives
  the same result.
