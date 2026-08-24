# Selector packs and the action interpreter

A selector pack is the data an adapter reads to drive one marketplace form, and the interpreter is the bounded machine that executes it.
The design specification requires both from the first chargeable milestone — every pack re-validated against its schema at load, every loop and match count bounded, total actions per job capped, fail-closed behaviour, and interpreter fuzzing as a flake check — and neither is buildable from that description alone, so the format, the vocabulary and the interpreter are specified here.

The threat model that shaped this is not the round-two one.
That analysis was written for a channel pushing signed instruction packs into every seller's browser, and most of it dissolves under server-side execution because a pack never leaves the founder's host, which makes signing, a TUF-shaped manifest, Rekor monitoring and an offline signing key defences against an operator-host compromise that already has everything else.
What survives is the CrowdStrike lesson: Rapid Response Content was explicitly not code but "a representation of fields and values", it was server-validated, and it still took down a global fleet because a bug in the Content Validator let problematic content through.
A pack is data, and data that drives a machine is validated as adversarially as code.

## The format

A pack is a JSON document, one per inventory per verb, versioned and stored in `adapter_selector_pack`.
Its Rust representation is a `serde` type in `tam-marketplace`, and the JSON Schema used for validation is generated from that type rather than hand-written, so the two cannot drift.

```rust
pub struct SelectorPack {
    pub adapter_version: AdapterVersion,
    pub inventory: InventoryId,
    pub verb: Verb,
    pub origin_allow_list: Vec<Origin>,
    pub form: FormId,
    pub expected_input_names: Vec<String>,
    pub actions: Vec<Action>,
}

pub struct Selector {
    pub primary: CssSelector,
    pub fallbacks: Vec<CssSelector>,
}
```

`adapter_version` is checked against the build's accepted range at load, and a pack outside it is refused with `FailureCode::AdapterVersionRejected` rather than executed.
`expected_input_names` is what the pre-flight form-schema assertion compares against, and it is part of the pack rather than a separate fixture so a selector change and a schema change move together.
`origin_allow_list` is enforced in Rust at the driver layer, below the interpreter, so a pack cannot widen it.

## The action vocabulary

The vocabulary is a closed enum, and closure is the security property: an action that does not exist cannot be expressed by any pack, valid or forged.

| Action | Effect | Bound |
|---|---|---|
| `Navigate { route }` | drive the browser to a route on an allow-listed origin | one navigation per action |
| `AssertPresent { selector }` | fail closed unless the element resolves | `MAX_SELECTOR_MATCHES` |
| `AssertAbsent { selector }` | fail closed if the element resolves | `MAX_SELECTOR_MATCHES` |
| `WaitFor { selector, budget }` | poll until present or the budget expires | `budget` capped at `WEBDRIVER_COMMAND_TIMEOUT` |
| `Fill { selector, field }` | type a projected `FieldKey` value into an input | value length capped at projection time |
| `Select { selector, option }` | choose a named option in a select | one option |
| `SetFiles { selector, role }` | attach payload, cover or preview files | `MAX_UPLOAD_FILE_BYTES` per file |
| `Click { selector }` | click a non-submitting control | one click |
| `ReadField { selector, field }` | read a value back for the diff | `MAX_SELECTOR_MATCHES` |
| `Submit { selector }` | the one action that can produce an ambiguous outcome | one per job |

There is no delete action, no script-evaluation action, no loop action and no conditional action.
The absence of a delete is the type-level form of the phase-one non-goal, so no configuration can express one.
The absence of loops and conditionals is what makes the action count statically bounded: a pack's action list is its execution trace, and `MAX_ACTIONS_PER_JOB` is checked at load as well as during execution, so an over-long pack is refused rather than truncated mid-form.

`Submit` is distinguished from `Click` because exactly one action in the system crosses the commit boundary, and a pack carrying two `Submit` actions is rejected at load.
That is the schema-level companion to the machine invariant that no `Submit` effect is ever emitted twice for one `WriteAttemptId`.

## The interpreter

The interpreter is a loop over `Vec<Action>` holding a `StepBudget` and nothing else.
It resolves each selector against the live document, decrements the budget, and returns a `Result<SubmitEvidence, AdapterError>` to the adapter, which returns it to the machine.
It never decides an outcome: a stringified `WebDriverError` is never permitted to classify anything, and every transport failure is classified at the `reqwest` layer below `thirtyfour` before the interpreter sees it.

Selector resolution is primary-then-fallbacks, and a resolution that succeeds on a fallback emits `FailureCode::SelectorResolvedViaFallback` as a non-fatal observation rather than an error.
That observation is the leading indicator the operations section relies on, available in week one rather than after two quarters, because a selector degraded to its fallback is a break that has not surfaced yet.
A selector matching more than `MAX_SELECTOR_MATCHES` elements is `SelectorAmbiguous` and fails closed, because an ambiguous selector is a failure class rather than a case to disambiguate at runtime.

Every post-authentication action asserts positively on an authenticated-only element rather than checking for the absence of a login form, because both dominant failure classes parse as success otherwise: a seller enabling multi-factor authentication yields a login page where a dashboard was expected, and a session expiring mid-upload yields a redirect to sign-in whose landing page is a cached edge 200.

## Validation and its tests

Every pack is re-validated against the generated schema at load, on every load, and not only when it is first written, because the CrowdStrike failure was a validator that passed content it should have refused rather than an unvalidated channel.
Validation checks the schema, the adapter-version range, the single-`Submit` rule, the action count against `MAX_ACTIONS_PER_JOB`, and that every origin in `origin_allow_list` is one the driver layer already permits for that inventory.

The interpreter is fuzzed against malformed packs as a flake check from the first milestone, with the property that no input document reaches a panic, an unbounded loop, a navigation outside the allow-list, or an action count above the cap.
Because the fuzz target is our own parser and our own interpreter rather than a third-party document parser, findings are fixable, which is precisely the distinction that sends archive and document parsing to a reaped subprocess under rlimits instead.

## Rollout

A selector change rolls canary, then five percent, then twenty-five percent, then one hundred percent of tenants, with automatic halt on a rise in the selector-failure rate.
A tenant may pin an adapter version and opt into a slower ring.
Release notes are published per pack version, because a customer who can see what changed does not open a ticket.
The per-marketplace gate is modelled on Mozilla's blocking process, which is version-scoped and non-overridable: when the upload form changes, create is disabled for affected adapter versions while edit keeps running, so the customer loses one capability for a day rather than the product.
