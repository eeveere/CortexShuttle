# WHO COUNTS AS THE SAME REPOSITORY?

## Milestone 3b: repository membership and sibling retrieval

## Why this increment exists

Milestone 3, "Context and repository history," split into 3a and 3b in
`docs/.shuttle-priv/dedicated-harness-plan.md` §12 (2026-09-21) once it became
clear the bounded admitted-edit v2 text-patch protocol was fully separable
from repository membership and cross-worktree Experience retrieval. 3a
shipped as `docs/plans/DO-WE-REALLY-HAVE-TO-REPLACE-THE-WHOLE-FILE.md`'s Chunks
0-7. 3b has never had a chunk plan. This is that plan.

The master plan already specifies the shape of this work in detail — §6.1
"Repository identity and membership," §6.2 "Shared history, separate current
state," §6.3 "Worktree retirement and proof," and three rows of §11
"Repository-grounded prerequisites" (`from_context` copying all selected
sources indiscriminately; workspace-scoped-only Experience search; failure
keys that assume the packet's workspace). This document turns that
specification into an ordered, gated delivery sequence the way the text-patch
plan did for the v2 protocol, continuing that plan's chunk numbering.

## Intended outcome

A worktree registers against a durable repository identity that sits above
its existing canonical-root workspace identity. A fresh sibling worktree of
the same repository can retrieve eligible verified Experience from other
registered worktrees as historical, read-only supplemental context — without
importing live source, task, or session state, and without the current
`HarnessHydrationRequest::from_context` behavior of copying every selected
source indiscriminately into a hydration request that only workspace-local
content should ever satisfy.

The implementation is complete only when:

1. A durable repository ID exists above workspace identity, discovered via
   the common Git directory / linked-worktree relationship (not a branch name
   or raw path hash), and stored with its observed location.
2. Workspace registration, membership changes, and their consistency rules
   are owned by explicit migrations, with revalidation triggered by moves,
   repaired Git metadata, or explicit resume — never invented retroactively
   for a pre-existing workspace.
3. The native context/hydration contract carries two separate inventories —
   current-code hydration candidates and historical references — and a
   historical reference can never satisfy a current-code hydration request.
4. A mixed packet (active-workspace code plus sibling Experience) hydrates
   the active code successfully with selection provenance preserved, and
   explicitly rejects hydration of the sibling entry from that same request
   rather than silently dropping or rebasing it.
5. An Experience's stored signature, workspace/session/task/episode identity,
   and evidence ownership stay immutable regardless of which worktree later
   retrieves it; cross-worktree matching uses a separately versioned
   repository-compatible rule, never a literal exact-match reuse of the
   signature.
6. Revoked membership and disputed/refuted/superseded Experience are excluded
   before every subsequent request, not only checked at registration time.
7. Removing a working directory retains its CortexWeave workspace and history
   so sibling worktrees can still explain eligible history; only explicit
   workspace deletion removes owned Experience, and the UI distinguishes the
   two.
8. Existing single-workspace behavior is unchanged by default. Repository-pool
   retrieval is an explicit opt-in extension; a workspace-only request still
   returns exactly what it does today.

## Scope and non-goals

In scope:

- repository ID discovery and registration via the Git common-directory /
  linked-worktree relationship;
- durable storage for repository identity, workspace membership, and a
  versioned sharing policy;
- the two-inventory hydration contract (current-code candidates vs.
  historical references) and its mixed-packet enforcement;
- repository-compatible Experience matching, kept distinct from the existing
  exact failure-signature match;
- worktree retirement semantics (retain history vs. explicit deletion);
- the full §6.3 test matrix: sibling sharing, isolated clones with identical
  remotes, divergent same-path code, detached HEAD, Windows path aliases and
  Linux case-insensitivity, membership revocation, disputed history, origin
  worktree removal, aggregate bounds, concurrent membership/lifecycle changes.

Out of scope for this increment:

- general repository-scoped Memory (§6.2 explicitly defers "knowledge from a
  different repository" — no automatic inclusion through this feature);
- merging or importing pre-existing per-worktree SQLite databases (§6.1 calls
  this a separately designed import workflow);
- the held-out-evaluation sibling-worktree research scenario (§10) — that
  scenario consumes this feature once built, it does not gate building it;
- any change to the admitted-edit v2 text-patch protocol (3a) itself.

## Proposed repository/membership contract

A repository record: a stable `repository_id`, the canonical common Git
directory it was discovered from, first-observed location, and a versioned
sharing-policy value. A membership record: `workspace_id`, `repository_id`,
`joined_at`, and a nullable `revoked_at` so revocation is a fact, not a
deletion.

The hydration contract needs two shapes where today there is one:
current-code candidates (today's shape — active workspace, selected
chunk/source ID, path/symbol, content/revision identity) and historical
references (origin workspace/session/task/episode, Experience/snapshot IDs,
historical authority, repository-sharing provenance). `from_context` needs a
compatible conversion path that supplies only active-workspace candidates to
hydration, so introducing correct origin IDs does not itself break today's
single-workspace behavior — the failure mode §6.1 warns against explicitly.

## Work chunks

### Chunk 8 — Repository identity and registration

Durable `repository_id` discovery from the Git common-directory/worktree
relationship; migration for repository and membership records; existing
canonical-root workspace registration extended to also record repository
membership, without changing current workspace-scoped behavior for any
existing caller.

Gate: registering two worktrees of the same repository yields one
`repository_id` and two memberships. An unrelated clone, fork, or copied
checkout with a matching remote URL, directory name, or commit does not
enroll. A moved worktree or repaired Git metadata triggers revalidation
rather than silently keeping stale membership.

### Chunk 9 — Two-inventory hydration contract

Split the hydration request into current-code candidates and historical
references as separate, explicitly typed shapes. A historical reference can
never satisfy a current-code hydration path — explicit rejection, not a
silent drop or rebase.

Gate: the mandatory mixed-packet contract test (§6.2) passes — active code
hydrates successfully with provenance preserved; a sibling Experience entry
in the same request is rejected, not silently dropped. Stale content hash,
revoked membership, and identical-path-with-divergent-content cases all fail
correctly.

### Chunk 10 — Repository-scoped Experience retrieval

Default Shuttle's request to the registered repository's eligible Experience
pool, behind an explicit opt-in; preserve a workspace-only request for
isolation and evaluation. The substrate resolves membership and applies one
bounded candidate/selection policy — Shuttle does not fan out searches or
rank merged histories itself. Lifecycle exclusions and authority ordering
apply before every bound.

Gate: sibling positive case (a fresh worktree retrieves another registered
worktree's eligible Experience) and isolation negative case (a workspace-only
request sees nothing cross-worktree) both pass. Revoked membership and
disputed/refuted/superseded Experience are excluded from a live request, not
only from initial selection.

### Chunk 11 — Repository-compatible matching and provenance labeling

A separately versioned repository-compatible match, distinct from the
existing exact failure-signature match that encodes workspace identity. Every
cross-worktree result is stamped with consuming workspace vs. source
workspace. Historical source snapshots and graph paths stay labeled
historical regardless of matching relative paths or symbols — never rebased
onto the consumer's branch by coincidence.

Gate: a repository-compatible match is never reported as an exact recurrence.
A historical snapshot retrieved into a sibling worktree still displays its
true origin workspace.

### Chunk 12 — Worktree retirement and the full test matrix

Worktree removal retains its CortexWeave workspace and history; only explicit
workspace deletion removes owned Experience, with the CLI/UI distinguishing
the two. Implement and pass the full §6.3 matrix on both platforms: sibling
sharing, isolated clones with identical remotes, divergent same-path code,
detached HEAD, Windows path aliases and Linux case-insensitivity, membership
revocation, disputed history, origin worktree removal, aggregate bounds, and
concurrent membership/lifecycle changes.

Gate: this closes Milestone 3b's exit evidence in
`docs/.shuttle-priv/dedicated-harness-plan.md` §12 in full — "sibling
positive and isolation/lifecycle/removal negative cases pass."

## Recommended task boundaries

Chunks 8 and 9 are where the real judgment calls live — durable identity that
must not be derivable from a branch name or path hash, and an inventory split
that has to add real separation without a single caller-visible regression.
This is the same category of subtle-correctness risk that the review after
Chunk 2b (3a) caught with G1/G2/G3 — recommend opus:high for those two, and an
adversarial review pass before treating either as closed. Chunks 10 and 11
are mechanical once the Chunk 8/9 contract is fixed; sonnet:high should carry
them. Chunk 12 is test-matrix breadth work — sonnet:high, with an opus:high
adversarial review once the full matrix is green, mirroring how 3a got
reviewed after its own tests passed rather than before.

## Open questions

- §6.1 says SQLite "owns registration, membership changes, and their
  consistency rules," and frames this as a CortexWeave-owned concern — but
  Shuttle and CortexWeave communicate only through a pinned Cargo dependency,
  not a live service. Does the repository/membership table belong in a new
  CortexWeave migration exposed through a new public API, or can it live in
  Shuttle's own migrations (as migration 0022's session ledger did for 3a)?
  This has to be settled before Chunk 8 starts; it changes which repository
  owns the schema.
- Does "repository-compatible matching" need its own `decisions.md` entry
  before implementation, the way S033 governs the v2 text-patch protocol,
  given it changes what counts as a legitimate failure-signature match across
  worktrees?
- §6.1 defers merging pre-existing per-worktree SQLite databases to a
  separate import workflow. Is there an actual pre-existing multi-worktree
  setup on the reference machine today that needs that workflow, or is
  Chunk 8-12 starting from a clean single-workspace state in practice?
