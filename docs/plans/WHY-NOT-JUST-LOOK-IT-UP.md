# WHY NOT JUST LOOK IT UP?

## K7: find guidance for admitted edit sessions

## Why this increment exists

K6 (2026-09-28, `.shuttle\k6-280926`) ran the whole admitted workflow against
emCP without a fault: plan, grant, edit, re-admission, both checks and the offer
all succeeded. The patch still missed the objective, and the operator rejected
it. The 4B worker read `AGENTS.md` from the top in 2,048-byte reads, used the
6,144-byte read budget by line 169, was forced to a patch-only turn, and edited
the one testing block it had seen, which was already correct. The two wrong spans
(lines 238-242 and 346-350) never reached it. See the K6 outcome in
[the plan-v2 record](../manual-qualifications/emcp-plan-v2.md).

By the K6 runbook's table this is a context or strategy failure, not a capability
failure. There are two ways to get the spans in front of the model: give it a
bigger read budget, or make it use `find_task_text`. This plan weighs them,
chooses guidance first, and orders the work.

## Budget or guidance

Facts, from the K6 journal (read with `mode=ro&immutable=1`) and `AGENTS.md` at
emCP `a652789` (355 lines, 11,643 bytes):

- **Find alone would have been enough.** `test:live` occurs 3 times in
  `AGENTS.md` (lines 24, 242 and 350), under the 8-match cap (`MAX_FIND_MATCHES`,
  `src/edit_session/text.rs:13`). One find returns at most 3 × 256 = 768 bytes.
  With two short reads around lines 238 and 343 to follow, the spans cost about
  1.5 KB of the 6,144 bytes and 4 of the 5 turns. A find counts as one read, and
  its excerpts count against read bytes (`observe_literal`, `text.rs:222`).
- **The guidance already exists, but it is soft.** The revision-2 system prompt
  (`ADMITTED_EDITING_V2_CONTEXT2_SYSTEM`, `src/llama.rs:31`) says find "with a
  distinctive literal is the cheaper way to reach text beyond a preview than
  reading lines in order". On turn 0 the worker read from line 1 regardless.
- **Reading from the top needs four S033 limits raised.** Line 238 starts at byte
  8,108 and the file ends at 11,643. At 2,048 bytes a read, reaching the first span
  takes 5 reads and the whole file 6. That means at least `MAX_EDIT_TURNS` 7,
  `MAX_EDIT_READS` 6 and `MAX_EDIT_READ_BYTES` 12,288
  (`src/edit_session.rs:35-37`), and a request and context bound above 24,000
  (`REQUEST_LIMIT`, `src/llama.rs:25`; `MAX_EDIT_CONTEXT_BYTES`,
  `src/edit_session.rs:40`). K6's turn 3, after three reads, was already 17,323
  bytes. The worker's `n_ctx` (65,280) is not the constraint. Any file larger than
  the new budget fails the same way, and the Chunk 7 follow-up plan listed this
  raise under "what I would NOT do".
- **Other things K6 showed.**
  - The session went patch-only with one read and two turns left, because the
    third read took every remaining byte.
  - Find excerpts start at the match, so a label on the line above it (such as
    `# Integration tests only` over `npm run test:live`) is not shown. A follow-up
    read is still needed.
  - The planner's summary already had the wrong framing ("the MCP stdio
    end-to-end test is the default behavior"), and the patch followed it.

| | Bigger read budget | Find guidance |
| --- | --- | --- |
| Reaches K6's spans | Yes, for files up to the new budget | Yes, if the model picks a usable literal |
| Scales to larger files | No | Yes |
| Cost | An S033 amendment: four limits, a bounds revision, larger prompts for a 4B model | Context revision 3: a prompt constant and perhaps per-turn fields; bounds unchanged |
| Main risk | Hides a strategy failure; more context for a small model to lose track of | The worker may ignore a directive too; labels above a match still need a read |
| Testable before code | No | Yes: prompt variants can be replayed against K6's requests |

**Recommendation: guidance first.** A budget raise becomes a candidate (K8, its
own S033 amendment) only if a later run shows the model using find and still
blocked by the budget.

## Intended outcome

On the same objective, constraint, profile and worker as K6, the edit transcript
reaches both target spans and the patch fixes them, with every S033 bound
unchanged and context revisions 1 and 2 byte-identical.

## Scope and non-goals

- No S033 limit rises.
- No change to the worker, the sampling settings or the objective. Seed and
  temperature are not varied to fish for a different patch.
- No change to the planner. Its framing is an open question below.
- The shape of find excerpts (including the line above a match) is deferred. It
  changes tool output, and the probe cannot test it at turn 0. Revisit it if a run
  finds the spans but mislabels them.

## Work chunks

⛔ marks an operator go/no-go.

### K7a: probe script (read-only, no Shuttle code)

`docs/manual-qualifications/k7-probe.py`, standard library only, like
`k6-transcript-dump.py`. It opens `.shuttle\k6-280926\journal.sqlite` with
`mode=ro&immutable=1` and takes the recorded POST bodies of turn 0 and turn 2
(`admitted_edit_turns` → `model_requests.intent_json` →
`serialized_request.exchanges[].body`). It builds variants by changing only the
named parts:

| Variant | Turn | Change |
| --- | --- | --- |
| V0 | 0 | None. Sanity check: should reproduce `read_task_text` from line 1 |
| V1 | 0 | The directive system prompt (below) |
| V2 | 0 | V1, plus the file's line count in the user JSON `files` entry |
| V3 | 2 | None. Baseline for a mid-session nudge |
| V4 | 2 | The directive prompt, the lines seen so far (`seen 1-107 of 355`), and a note that the next read may use the rest of the read bytes |

It asserts that each body stays within 24,000 bytes, POSTs each variant 10 times
to the endpoint in the saved profile, and records the finish reason, the first
tool and its arguments. For a find, it also records whether the literal occurs
within 5 lines of 238-242 or 346-350 in `AGENTS.md` at `a652789` (read with
`git show`). Output goes to `.shuttle\k7-probe\results.jsonl`, with a summary
table. Committing the script needs `git add -f`, because
`docs/manual-qualifications` is ignored.

Limitation: at temperature 0, repeats measure the server's nondeterminism, not a
rate.

**K7a run ⛔ operator.** The worker is not restarted or reconfigured. The adoption
rule is fixed before running: a variant's change goes into revision 3 if it
produces a find on that turn in a clear majority of its repeats, with no more
length stops than its baseline. If V1 does no better than V0, stop. Revision 3 is
not built, and the next step is an operator decision.

### K7b: S035 decision

Context revision 3 for admitted edit sessions: the directive prompt plus whatever
K7a adopted, with bounds unchanged (`bounds_revision` 1) and revisions 1 and 2
pinned. Independent review. ⛔ operator accepts.

### K7c: implementation

Follow S034's pattern:

- a revision-3 constant next to `CONTEXT_REVISION_REFERENCE`
  (`src/edit_session.rs:44`);
- the new system prompt next to `ADMITTED_EDITING_V2_CONTEXT2_SYSTEM`
  (`src/llama.rs:31`, selected at `src/llama.rs:613-621`);
- the revision match in `validate` (`src/edit_session.rs:540`);
- the equality checks at `src/llama.rs:181`, `src/edit_review.rs:168` and
  `src/edit_session.rs:520`, widened so revision 3 keeps revision 2's reference
  context;
- any per-turn fields K7a adopted.

Tests pin the revision-1 and revision-2 prompts and wire bytes unchanged, pin the
revision-3 prompt, and show that any new fields fit the R1 next-turn check. Gates:
`cargo fmt --check`, warnings-denied Clippy and locked tests on Windows and Docker
Linux, plus an independent review. ⛔ operator accepts and commits.

### K7d: live run ⛔ operator

The K6 runbook with a new state directory and the same objective, constraint
(hash-checked), profile (digest `c02198cf…`) and worker (`b10278-d52ec04a6`).
The runbook replaces its `<…>` placeholders with captured variables (`$ctx`,
`$adm`, `$offer`), which broke three commands in K6. The transcript dump uses the
same spans and literals. The result is recorded as a "K7 outcome" in the plan-v2
record.

## How to read the K7d result

| What you see | What it means |
| --- | --- |
| Find used, spans reached, patch right, checks pass | Judge it on its merits and record accept or reject |
| Find used, spans reached, patch wrong | Capability with sufficient context: the input to the larger-worker experiment |
| Find used, but the budget ran out before the spans | Evidence for a K8 budget amendment |
| Still reads from the top | Guidance does not move this worker |

It is one sample either way.

## Draft directive prompt

This replaces the find sentence in the revision-2 text:

> Both spend a small fixed budget that cannot cover a long file. When the text you
> need to change is not in a preview, first call find_task_text with a short
> literal from the objective, the constraints or the reference files, such as a
> command or script name, then read a few lines around the matches it reports. Do
> not read a long file from the top.

## Open questions

- Should the planner's framing be addressed? K6's plan summary was wrong before
  the edit started, and the edit model followed it even though the prompt calls it
  an unverified note.
- Should find show the line above a match?
- Should a patch-only turn be forced while reads remain because the byte budget
  ran out?
