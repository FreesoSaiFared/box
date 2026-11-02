# Mixed guest recipes — native execution, CUA and observations

Owner's implementation directive (2026-11-02): build coherent native vertical
slices in one go rather than breaking implementation into toy test milestones;
use testing to verify the complete slice, not to justify narrowing it.
Defer *new* security-hardening work to a separately requested phase; preserve
existing authentication, guest boundaries and user modifications unchanged.
Challenge conventionalist pull toward treating CUA batching as the final
architecture: native code, observations, and semantic evidence are first-class.

The new POST /v1/recipe endpoint is a one-request program runner, extending
(not replacing) the existing grok-box /v1/cua/recipe, /v1/exec and /v1/files
surfaces. It keeps the existing Bearer token and workspace rules.

## Executable steps

A recipe holds 1–256 uniquely identified steps, fully linted before any
effects. Supported ops: exec, write_file, read_file, cua, assert, wait.
Exec accepts a shell string or an argv array and returns native stdout,
stderr and exit code. A CUA step embeds the *existing* RecipeRequest,
including per-step input/page observations, settle, screenshots and videos.
Content inputs can reference a previous step through a JSON pointer:
{ "from": "generate", "pointer": "/stdout" }. References must point backward.
Nothing inside the guest program calls a language model.

A sample payload lives in examples/guest-program.json. For a TEST guest:

    curl -fsS -X POST "$BOX_EXEC_URL/v1/recipe" \
       -H "Authorization: Bearer $BOX_TOKEN" \
       -H "Content-Type: application/json" \
       --data-binary @examples/guest-program.json

## Observation levels, not inflated success claims

- delivered: the execution surface accepted/performed a step. This is
  not a claim about the intended semantic result.
- observed: a read/inspection returned evidence. Missing fields are not
  interpreted as falsity or success.
- verified: a declared assertion matched a previous observation.

Only explicit assert steps claim verification. Zero assertions equals zero
semantic checks, even if ok=true. You can assert JSON pointers inside the
entire CUA recipe output, including steps/1/observed/url_after, or the
content read from a file. Assertions support equals, contains and present.
An absent field fails present:true. A present field with value null
remains present for that purpose.

## Durable pending-effect handling

Optional journal writes each step result using atomic rename, and records
a pending step before any effect. A resumed identical plan reuses completed
steps. A failed step can be retried. If the process dies after an effect but
before its completion receipt, resume returns needs_reconciliation rather
than replaying an unknown effect. This is NOT an exactly-once guarantee
on remote systems; reconciliation requires independent evidence.
Use distinct journal paths for concurrent jobs.

CUA pointer actions remain sequential (one pointer). The mix of native
programs and screen recipes can be exposed as one execution surface
beneath a semantic compiler, which may schedule independent API effects
as a DAG. The guest program is not itself a predicate-language compiler.

## Conventional-pull falsification experiments

Compare the same tasks, not just HTTP time, across:
1. Model-driven single-action tool loops with observation at every turn.
2. One code-authored JavaScript/CLI procedure.
3. One grok-box fixed-coordinate CUA recipe (observe off/input/page).
4. One mixed guest program with native code, assertions and journals.
5. A Reverse-WebMCP semantic DAG dispatching guest programs as leaves.

For each arm record model calls/tokens, HTTP/MCP calls, wall time,
observation overhead and bytes, materialized artifacts, independently
verified final-state predicates, errors and ambiguous effects. Add
layout drift, lost return/navigation, wrong input focus, external
state mutation, and crash-between-effect-and-journal-completion.

A successful gesture is not a successful objective. A declared assertion
is not proof of an unobserved world. A compiled plan needs a validated
semantic model; otherwise runtime efficiency can amplify wrong premises.

## Limits

The current implementation does not implement rwmcp's semantic compiler,
automatic WebMCP discovery, a parallel DAG scheduler or a model planner.
It exposes a programmable, inspectable native guest endpoint for those
higher layers. Build and deploy are distinct; this file is not a
deployment receipt.
