# Plan Studio — overview and surface architecture

**Working title:** Plan Studio

## Intent

Plan Studio is a companion planning surface for Shuttle. It turns a vague task
into a reviewable, bounded execution proposal without gaining authority to modify
the repository, run a command, grant permission, accept work, or finalize a task.

The motivating direction is deliberately broader than a form inside Shuttle:

> “or even as a module or a full application that's compatible with
> Shuttle/Weave/VEEP(extensibility wise) that can be used for deep and
> comprehensive planning work.”

The first integration should nevertheless feel close to the task being planned:

> “that Shuttle call that can open a TUI plan builder+reviewer mod window.”

Plan Studio therefore has two valid presentations over one contract:

- a focused in-terminal plan builder/reviewer launched from `shuttle task`; and
- a richer standalone or host-embedded application for research-heavy planning.

Neither presentation changes Shuttle’s role as the durable execution and review
authority.

## What it solves

Today, a human must construct a `VerificationPlan` JSON file before creating a
general Shuttle task. That format is intentionally strict because it binds exact
inputs, process identities, arguments, environment, resource limits, exclusions,
and waivers. It is correct but too low-level to be the primary planning experience.

Plan Studio should help an operator:

1. clarify the objective and constraints;
2. explore relevant repository files and existing tests;
3. identify a deliberately narrow change surface;
4. propose deterministic verification checks and their required inputs;
5. make limitations, uncertain assumptions, and alternatives visible; and
6. emit a canonical, human-approved plan that Shuttle can admit.

It is not a coding agent, background runner, autonomous task scheduler, shell,
or approval bypass.

## Core architectural boundary

```text
                research, drafting, alternatives
                        ┌─────────────┐
                        │ Plan Studio │
                        └──────┬──────┘
                               │ draft + rationale
                               v
                   canonical reviewed plan artifact
                               │
                               v
                        ┌─────────────┐
                        │   Shuttle   │
                        └──────┬──────┘
          admission, permissions, execution, evidence, decision, finalization
                               │
                               v
                         CortexWeave
                    factual/native lifecycle record
```

Plan Studio may recommend; Shuttle validates and acts only after explicit human
approval. CortexWeave continues to receive factual lifecycle outcomes from
Shuttle, not speculative planning claims.

## Artifact model

Plan Studio should produce two related artifacts.

### 1. Execution plan — authoritative input to Shuttle

This is the existing versioned `VerificationPlan` shape, extended only through
intentional schema evolution. It contains:

- named checks with exact executable identity, arguments, working directory,
  complete environment, and bounded limits;
- declared source, test, runner-configuration, manifest, and lockfile inputs;
- explicit exclusions with reasons; and
- explicit waivers with reasons.

Before Shuttle saves task intake, it canonicalizes and validates this artifact,
derives its revision, and presents the exact resulting contract for approval.
No planner-supplied hash, path, command, or environment is trusted merely because
it appeared in a draft.

### 2. Planning dossier — non-authoritative context

This can be richer and evolve independently. It may include:

- objective restatement and constraints;
- repository observations and bounded file references;
- proposed change surface and non-goals;
- rationale for each declared input/check;
- alternatives considered, assumptions, risks, and limitations;
- links to issues, design documents, or research evidence; and
- an explicit human review state.

The dossier helps people understand *why* a plan exists. It cannot authorize
execution or be silently converted into an executable command.

## Surface-level components

| Component | Responsibility | May not do |
| --- | --- | --- |
| Task brief editor | Capture objective, constraints, success conditions, and non-goals. | Start a model, command, or edit. |
| Repository explorer | Read bounded, user-authorized repository metadata/files and identify candidates. | Treat discovery as permission to include or change a file. |
| Plan composer | Draft inputs, checks, exclusions, waivers, and dossier rationale. | Resolve executables through ambient `PATH` or inherit secrets. |
| Local inspector | Independently canonicalize paths, resolve an explicitly selected executable, hash it, and show its proposed environment/limits. | Launch the check as a side effect of inspection. |
| Review diff | Compare plan revisions and show every authority-relevant change. | Auto-approve a changed plan. |
| Shuttle bridge | Submit a reviewed canonical plan to `task-intake` and open a task workspace. | Admit, grant, edit, verify, accept, or finalize without Shuttle’s normal confirmations. |
| Extension host | Permit alternate research/planning providers through typed capabilities. | Let an extension obtain blanket filesystem, shell, model, or approval authority. |

## Integration contract

The first contract should be small and local:

```text
openPlanStudio(request) -> draft
validateDraft(draft, localInspection) -> canonicalPlan + diagnostics
reviewCanonicalPlan(canonicalPlan, dossier) -> explicit human choice
submitApprovedPlan(canonicalPlan) -> Shuttle task intake
```

`openPlanStudio` receives a user-selected workspace and a bounded task brief. It
does not receive a pre-authorized shell, an unrestricted file crawler, or stored
credentials. The host can offer read-only repository capabilities incrementally.

`submitApprovedPlan` is the first point where Shuttle persists a task. It must
continue to reject malformed/oversized plans, unsafe paths, unknown executables,
missing hashes, ambiguous input declarations, and changed task identities.

For VEEP or other future host ecosystems, expose these as capability-based typed
operations rather than coupling the plan schema to a particular UI framework or
application runtime. Compatibility means a host can render or extend planning;
it does not mean a host can bypass Shuttle’s durable controls.

## Initial user experience

1. In `shuttle task`, the operator chooses **Create/review plan**.
2. Plan Studio opens a terminal modal initially; a standalone window may implement
   the same contract later.
3. The operator enters the objective and constraints, then reviews suggested
   repository inputs and checks.
4. For each command, the local inspector displays the exact executable, BLAKE3
   identity, arguments, environment, directory, timeout, and output bound.
5. Plan Studio renders a canonical JSON preview plus a readable summary and
   dossier limitations.
6. The operator explicitly approves the canonical plan.
7. Shuttle saves immutable intake, displays its derived revision, and begins the
   existing preflight → admission → planning → permission → edit → verification →
   review flow.

If any locally inspected identity changes before approval, the draft is stale and
must be inspected again. Saving a plan remains descriptive; it does not dispatch
a model or command.

## Trust and safety rules

- Human review is the sole source of execution authority.
- A drafted command is data, never an instruction to run it.
- Executables are selected explicitly and hash-bound; ambient `PATH` is not an
  authority source.
- The child environment is explicit and minimal; secrets are neither displayed
  nor inherited by default.
- Repository reads are bounded, declared, and visible in the dossier.
- Extensions operate through least-privilege capabilities and cannot self-grant
  more capabilities.
- The final execution plan and dossier preserve provenance separately: the plan
  is authority-relevant; the dossier is explanatory.
- Plan revisions require renewed review. A later draft cannot reinterpret an
  earlier approval.

## Suggested delivery sequence

### Phase PS-1 — Local canonical-plan reviewer

Build a read-only plan viewer/editor in Shuttle’s terminal. It imports an existing
JSON plan, validates it, independently inspects executable identity, renders the
canonical result, and saves only after explicit confirmation. No repository
research automation, model use, command dispatch, or new plugin runtime.

### Phase PS-2 — Guided local plan composer

Add bounded workspace exploration and forms for inputs, checks, exclusions, and
waivers. Generate a draft from operator choices, but preserve the PS-1 review and
canonicalization boundary. Include plan-diff review and stale-identity detection.

### Phase PS-3 — Planning dossier and research adapters

Add the separate dossier artifact and opt-in, capability-scoped research adapters.
An adapter may summarize supplied repository context or external references, but
its output remains untrusted draft material. Add clear provenance and limitation
rendering.

### Phase PS-4 — Extensible host integration

Expose the typed Plan Studio contract to compatible hosts such as VEEP. Keep the
terminal modal as a reference implementation. Validate that extensions cannot
invoke execution, alter an approved plan, or access undeclared data.

## Open design questions

- What repository-reading budget is practical for an interactive plan session?
- Should a dossier be stored in Shuttle’s journal, a separate local database, or
  an external host-owned store with a content hash retained by Shuttle?
- Which extension capabilities should exist first: read-only repository search,
  issue tracker lookup, document retrieval, or local-model summarization?
- How should an operator intentionally carry a plan between a standalone Plan
  Studio session and a Shuttle task while preserving provenance and freshness?
- What is the minimal terminal modal that remains pleasant without delaying the
  richer standalone experience?

## Non-goals for the first version

- automatic task acceptance or Experience creation;
- autonomous code editing or command execution;
- background monitoring or scheduling;
- transparent use of cloud services or private credentials; and
- replacing Shuttle’s existing durable journal, native outbox, or acceptance
  lifecycle.
