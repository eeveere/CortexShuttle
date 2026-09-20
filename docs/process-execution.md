# Process execution contract

The first host executor uses the existing controller's prepared → started →
observed-result/outbox sequence. `shuttle process` executes or recovers one exact
approved specification. It creates factual `ExternalToolFinished` Events through
the pinned CortexWeave service. A successful exit is not user acceptance,
qualified test evidence, episode completion, or an Experience claim.

## Scope and authorization

The specification records an absolute executable and its BLAKE3 content hash,
an argument vector, a workspace-relative working directory, the complete child
environment, and execution/output limits. A grant covers the hash of that entire
specification plus the canonical workspace root and declared input manifest.
Changing arguments, environment, directory, executable, limits, root, inputs, or
grant revision invalidates prepared work. The controller checks again after
restart; the executor also rechecks at the launch boundary.

There is no implicit PATH search or shell. Windows requires a native `.exe`;
batch files require an explicitly selected and granted shell. Native Windows
arguments use Microsoft CRT quoting. Programs with custom argument parsers,
including shells, still interpret their own arguments. The child gets EOF on
stdin, separate output pipes, and no inherited terminal. Only explicitly selected
environment entries are passed; the spec helper includes SystemRoot on Windows.

The input manifest declares 1–256 existing files totaling at most 16 MiB.
The precondition includes root identity, ordered manifest names, and file bytes.
Symlinks, Windows reparse points, parent traversal, and Windows network/device
paths are rejected. This detects ordinary input changes. It is not an atomic
snapshot, a complete verification manifest, or protection against hostile
concurrent writers, hard-link aliases, or unlisted dependencies. The executable
hash does not identify its dynamic libraries, scripts, or entire runtime.

Commands execute with the host user's filesystem and network access. Working
directory and permission grants are not an operating-system sandbox. Process
ownership covers descendants created by the command, not effects delegated to
external services. Commands must not kill or alter Shuttle or its Linux supervisor.

## Lifetime and output

- Windows 10+: an unnamed Job Object has kill-on-close enabled and is assigned
  atomically through `PROC_THREAD_ATTRIBUTE_JOB_LIST`. Only stdin/stdout/stderr
  handles are inherited. The job handle is private and no breakaway is enabled.
  There is no create-before-job-assignment interval. Root exit does not finish an
  action while descendants or output writers remain.
  Embedding hosts must also use explicit handle lists for concurrent launches;
  legacy inherit-all launches can retain another invocation's output handles.
  The application runs one active task. The tests isolate legacy crash-fixture
  launches and separately exercise concurrent native executor launches.
- Linux: the installed Shuttle binary runs a dedicated subreaper with a private
  control pipe and status socket. Parent death closes the control pipe. Cleanup
  kills direct children and repeatedly reaps/adopts descendants until `ECHILD`,
  including descendants that leave the original process group. The supervisor
  requires Linux `/proc` and `PR_SET_CHILD_SUBREAPER`; it does not require a
  privileged container or a host PID namespace. A supervisor failure without a
  final cleanup receipt leaves completion unknown.
- Cancellation is one-shot and kills the invocation's descendants. It does not
  roll back file edits or any other effects. A dropped execution future requests
  cleanup but leaves its action started, which becomes unknown on reopening.
- Default timeout is 120 seconds. Optional output-inactivity timeout is disabled
  by default: a silent compiler is not necessarily stalled. Cleanup has a separate
  five-second allowance. Failure to confirm cleanup never produces a terminal
  success/failure/cancellation receipt.
  These are polling deadlines, not hard real-time limits: a blocking operating-
  system launch or filesystem call can delay observation. Actual elapsed time,
  including such overruns, is charged when a result is committed.
- Each stream retains up to 4,096 raw bytes, records its observed byte count and
  truncation, and continues draining after retention is full. Work per polling
  tick is bounded, so continuous output cannot starve deadlines. Raw byte arrays
  avoid interpreting terminal escapes and preserve non-UTF-8 output. The entire
  JSON artifact fits the journal's 65,536-byte limit; discarded output is not
  available elsewhere as an unlimited artifact.

`ProcessReport` records root PID, actual exit code or Linux termination signal,
stop reason, elapsed milliseconds, stdout/stderr, and whether the tree stopped.
Launch failure is an observed failure without a root PID. Cancellation produces
`cancelled`; deadline/inactivity termination produces `failed` with its actual
reason. Normal nonzero exit also produces `failed`. An observed normal completion
can win a concurrent cancellation race. Uncertain effects, post-run input identity
failure, missing supervisor cleanup, or failed result commits block replay.

## Durable limits and recovery

Migration 0002 adds a one-hour cumulative process-time budget. Starting a command
reserves its whole timeout plus cleanup allowance in the same transaction as its
started marker. Completion replaces that reservation with observed elapsed time
in the result/artifact/outbox transaction. Restart after an uncertain result
retains the whole reservation. Replaying a saved result cannot charge/refund it
again. Insufficient budget pauses before dispatch; only a future explicit user
budget-extension operation may replenish it.

Three consecutive successful identical process actions with unchanged declared
inputs and complete identical output pause for direction. The fingerprint omits
action IDs, PIDs, elapsed time, and delivery receipts. Changed output or inputs
break the repetition; truncated output cannot establish repetition. This count
survives restart through action history. Failed/cancelled processes pause in the
same transaction as their results, and delivering pending Events does not unpause
them. [Run accounting](run-accounting.md) additionally covers model/context time,
six actions without new evidence, and one caller-directed bounded replan.

The CLI reuses the same saved action ID for the same state directory. It prints
an existing observation without running that action again. Unknown actions stay
blocked. A distinct retry needs explicit authorization and a separate run; a
guided recovery UI and linked retry receipts remain future work.

## Trying one command

Inspect the generated specification before approving it. Use an existing declared
input such as `Cargo.toml`; choose a dedicated state directory for this run.

```powershell
cargo run --locked -- process-spec --executable C:\Windows\System32\hostname.exe
```

Save that JSON as `command.json` (UTF-8), then run:

```powershell
cargo run --locked -- process --workspace C:\dev\CortexShuttle --state-dir .shuttle/command-demo --spec command.json --input Cargo.toml --approve-host-execution
```

On Linux, generate the spec with an actual executable such as `/usr/bin/printf`
and arguments after `--`. Resolve symlinks using the spec helper. Ctrl+C requests
cancellation while the command is running. The CLI returns nonzero for a failed,
cancelled, or uncertain execution, and for undelivered provenance.

## Platform references

The Windows creation and lifetime choices follow Microsoft's
[process attributes](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute)
and [Job Object contract](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects).
Qualification results and remaining native terminal checks are recorded in
[implementation status](implementation-status.md).
