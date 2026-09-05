---
title: Triage rules for the shared-sheet sweep
---

Working rules for deciding whether a CSS rule in the shared sheets is dead, and for the related class of defect where an absent value is rendered as a figure.
Written during the console shell wave on 2026-09-05, when both questions came up repeatedly and were answered ad hoc each time.
It is a working note rather than charter: it records a method that worked, and promoting any of it into the engineering charter is a later deliberate edit.

## Deciding that a CSS rule is dead

The shared sheets are `web/src/app.css` and the three under `web/src/lib/styles/`.
The question is which rules have no consumer, and it is answered in three rungs of increasing cost.

A zero from a scan of class attributes is a candidate.
A zero from that scan together with a scan of string literals is a fact, but only for a distinctively named class.
For a common-named class nothing short of removing the rule, rebuilding and comparing the rendered pages settles it, so that rung is for the few worth spending it on rather than for a list.

An attribute scan alone cannot be trusted for a zero, because classes are composed in script and passed as variables.
`web/src/lib/Button.svelte:43` builds `cta`, `btn`, `add`, `quiet`, `danger` and `small` into a string and renders `class={classes}`, so an attribute scan reports `small` and `danger` at two uses each when they are on every button in the console.
`web/src/lib/StatusPill.svelte:14` renders `class="status {tone}"`, and `web/src/lib/outcome.ts:15` carries `tone: 'seg-ok'` and eight siblings, so nine live rules are invisible to an attribute scan and are recovered only by the literal pass.

One refinement was proposed and rejected, and it is recorded because it is plausible.
The proposal was to treat a string literal as a possible class only when its own module also contains a dynamic class expression, on the reasoning that a module with no such expression cannot contribute a class name.
That is unsound, because a module can export the string and a different module can turn it into a class.
`web/src/lib/inventory.ts:141` contains no class attribute at all and exports `STATE_TONE`, whose values reach a class attribute in two components.
The sound repair follows imports, and then clears almost nothing, because nearly everything under `web/src/lib` is transitively imported by something with a dynamic class expression.

What actually discriminates is the shape of the name, and it needs no import graph.
A hyphenated compound is a class name or nothing, because ordinary code does not hold `foot-note` as data.
A bare English word is ambiguous, because ordinary code holds `ok`, `warn`, `bad`, `small`, `t`, `s` and `row` constantly.
So the working method is to run the blunt scan, read its output with that distinction in mind, and send every single-word candidate to the third rung.

Two kinds of dead are not the same kind of claim.
A class that has lost one consumer is a question whose answer changes as other work lands.
A rule with no consumer anywhere in the tree is a fact.
Carry the count on every question rather than recording it as "still in use", because one consumer in a file that is due to be redrawn is a different prospect from forty-one across the tree, and a list that flattens them into the same row cannot be sorted by how close each is to becoming a fact.
Only facts are deleted on sight, and a scan is a photograph rather than a fact about the future, so it is re-run immediately before any deletion is applied.

The two kinds also differ in how long they stay true, which is a stronger statement than differing in confidence.
A count of one is a question whose answer moves as work lands.
A count of zero is a fact about the tree it was read from, and it stops being one the moment an unlanded change adds a consumer, so a zero taken mid-wave is a candidate however it looked when it was taken.
That gives the method a precondition and not only a procedure: the scan that a deletion rests on has to be run after the last change lands, and every list produced before then records the tree it was read from rather than only the count.

There is a second kind of confidence that does survive the wave, and it is worth marking separately where it applies.
A rule that is the remnant of a layout the product no longer has cannot plausibly acquire a consumer, because nothing being built would reach for it.
That is a different claim from "the number was zero when I looked", and an entry resting on it can be deleted on a mid-wave reading where an entry resting on the count cannot.

## Rendering an absent value as a figure

A figure computed from a query's data with `?? 0` becomes a claim the reader believes, and the zero is wrong precisely because it is plausible.
An implausible wrong value gets questioned; a plausible one does not, so a reader whose data failed to load sees exactly what a reader with genuinely none sees, and only one of them is being told the truth.
The repair is a third answer for "not known" rather than a boolean over a fabricated zero.

This console already refuses plausible substitutes elsewhere, which is what makes the repair a pattern rather than a special case.
`ConnectionStatus` carries `checking` rather than folding an unverified link into connected or disconnected.
`ListingStanding` carries `other` so an unrecognised state is counted rather than dropped.
`readPrice` answers `unreadable` rather than guessing free.

Where the repair is put decides whether anything can check it.
A substitution written in a template is reachable only by rendering the page, so the cheapest honest place for it is a function that takes the read state and answers the absent case itself, which a unit test can then hold: one assertion that an unread collection and an empty one do not produce the same figure is enough, and it costs milliseconds rather than a browser.
The same move made `Tab.count`'s rendering testable by lifting it out of the markup into a named function, and it is the reason the nullable count could be pinned at all.
A rendered assertion earns its keep on what is left — the substitutions that happen in the template rather than in a function, which is where `?? 0` and `?? []` usually are — and images belong to the tier below that again.

Not every `?? 0` is this defect.
Initialising an accumulator is correct, a map lookup that genuinely means none is correct, and a zero standing in for an absent series inside a `Math.max` that computes a scale is correct.
Thirteen of the fifteen occurrences in the client are sound for one of those reasons.

### Correct handling nearby is a reason to look harder, not to relax

Scanning a list of occurrences, the instinct is to clear the ones sitting beside code that plainly handles the absent case and to spend the expensive rung on the ones that look unguarded.
That instinct is backwards.

An occurrence beside correct handling of the same absence is more suspicious than an unguarded one.
An unguarded occurrence may simply have no absent case to handle.
An occurrence beside correct handling has a proven absent case and a branch that forgot it, and the correct handling nearby is exactly what stops a reviewer looking harder at the branch that did not.

The worked example is in the operator overview, where one card reads `ledger === undefined ? '—' : parked` for its value and "the ledger has not been read" for its sub-line, then computes its icon and tone from `(ledger?.failed ?? 0) > 0` in the next expression.
An unread ledger therefore renders an em dash, the words "the ledger has not been read", and a green tick, together, and on an overview the tick is what gets scanned.

## Two ways a correct answer fails to spread

Both were observed several times over one day, and they call for different habits, so they are named separately.

### A fix that does not travel

The correct form is already in the tree, written by someone who understood it, and the defect is a site nobody connected to it.

`web/src/lib/admin.ts:100` reads `identity === null ? null : (identity.get(day) ?? 0)`, which separates "no data at all" from "a day with none", and sat one file away from pages that fabricate a zero.
`justfile:110` records that `deny` uses bash rather than sh because pipefail is what stops a failed `cargo metadata` from passing the check vacuously, and the vocabulary gate in `web-check` was written without it, so a crate that did not compile reported that the vocabulary was stale and pointed at a recipe that would have truncated a correct file.
`@tanstack/svelte-query` at 6.1.48 passes a `createQueries` combined result through `createRawRef`, which proxies a plain object and copies values in per key, so a bare `Map` returned from `combine` arrives with no keys and the first `.get` throws; the wrapped form was already correct in two files while three others passed a bare `Map`.
What that cost was worse than a blank route in at least one place: on the resource detail the throw was caught inside a row loop, the marketplaces panel fell through to reporting that the resource carried no mapping, and the page then offered to cross-list it onto marketplaces it was already listed on.
So an untravelled fix produced a wrong action offered to a seller rather than only a page that failed to render.
The `Tab` count that could not express an unknown was the same shape again, with the correct pattern sitting in `admin.ts`.

A comment can fail to travel in the same way, and it is the worst of the variants, because a stale comment argues against the change somebody is about to make rather than merely failing to prompt it.
The asymmetry is that a fix reaches code by being needed there, while nothing makes it reach the sentence that justified the old shape.
So a comment justifying a constraint should name the capability the constraint rests on rather than the consequence, because only the name turns the comment up when the capability changes.
One in this tree explained that a form's tabs had to stay separate from the counted tab bar since a form had "no figure to count"; widening `Tab.count` to accept a null removed the reason and touched nothing that pointed back at the sentence, which went on asserting the constraint for hours after it had lapsed.

The habit is to search for the correct form rather than the broken one, and it is the cheaper search.
`?? 0` has fifteen occurrences and thirteen are sound, while `identity === null ? null :` has one occurrence and it is the whole answer.
`set -euo pipefail` has two occurrences and the third recipe is the finding.
The wrapped-`Map` form has three occurrences today, at `web/src/lib/pages/resources/ResourceForm.svelte:69`, `web/src/lib/pages/resources/ResourceDetail.svelte:109` and `web/src/lib/pages/templates/MappingTab.svelte:53`, and would have found the sites that lacked it in one search, where "pages that render blank" is not a search at all.
So: after fixing something, search for the form just written, not the one just deleted.

### A rule generalised from the visible cases

The other direction is a mechanism inferred from the occurrences in front of you and never tested against the ones that are not.

An empty state's glyph sat left while its heading centred, and the mechanism was inferred from a sibling rule as "flex children do not inherit `text-align`".
That gave the right fix for the wrong reason: the actual cause is that Tailwind's preflight sets `svg { display: block }`, so a blockified glyph in a centred text box ignores the alignment, and the general form is that any bare icon dropped into a centred text box needs a flex or grid wrapper.
The module-scope refinement above is the same shape, generalised from two modules that happened to hold their own class expressions and falsified by the first module that exports one instead.

The habit here is the opposite of the first: before believing a rule, go looking for the case that would break it.
The `Map` case is the strongest argument for describing a mechanism rather than a symptom, because its symptom pointed nowhere near its cause: the page under it was being investigated as a reactivity problem, on the evidence of a derived value reading two at page scope and zero inside a snippet, and no amount of reasoning from that observation reaches a proxy that copies own enumerable keys.
Neither habit substitutes for the other, and a reader who takes only the first will search after every fix and still ship a wrong rule.

## Discharge by construction where it is available

Where a rule can be made unavailable to break it should be, rather than remembered.
Typing a function's parameter as the narrower of two shapes means a later edit cannot reach for the wider one and still compile.
`set -euo pipefail` in a shebang recipe means a gate cannot pass vacuously on a command that failed upstream.
A guard written at a call site is the weaker kind, still a decision and still forgettable at the next call site, which is why one is worth a name and a test rather than a comment.

A shared lane must name what it caught.
The window in which a change spanning two files leaves the shared tree red cannot be closed and is not the defect; a failure that does not say what failed is.
A type check that names the file and the line costs a reader one message to attribute; a gate that names nothing cost most of a day.

## A page sheet and the shell share one namespace

A page sheet and the shared sheets are the same namespace, and a name declared in one reaches markup rendered by the other.
Both directions have now been observed, neither is caught by anything today, and they fail differently enough to be worth naming separately.

The first direction is a page reaching the shell.
`account.css` declared a bare `.plan` for a pricing card, and the console's top bar renders `<span class="plan">` for the host caption beneath the organisation name.
Account routes load that sheet, so the pricing card's border and padding wrapped the shell's caption on every route in the section.
Renaming it to `.acct-plan` settles it; scoping it to `.plan-grid .plan` would not, because the bare name stays free and the next slice to declare one reopens the same defect against the same caption.

The second direction is the shell reaching a page, and it goes unnoticed for longer because the page looks correct.
The shared sheets declare a bare `.top` and a bare `.tick`, and a page whose own markup reused those names had its card header given the shell's padding and bottom border, and its list ticks pushed five pixels low.
What made it survive is that the page carried a `padding: 0; border: 0` reset that cancelled the first of them, written by an author who could not have said what it was cancelling.
A reset whose reason nobody can state is the signature of this defect, and it is worth treating as one wherever it turns up.

So the rule is two-sided, and only one side of it is the prefixing convention.
A page-local class is prefixed with the slice's own short name, as `res-`, `mk-`, `tpl-`, `acct-` and `import-` already are.
A page must also not declare a class whose bare name the shared sheets already declare, prefixed or not.
The first half stops a page reaching the shell; only the second stops the shell reaching the page, which is why the convention alone left both defects standing.

The check is cheap, needs no browser, and runs in both directions.
For every top-level class a page sheet declares, ask whether a shared sheet declares it too; for every class a shell component renders, ask whether any page sheet declares it.
Either question is a comparison of two name sets, which is the shape of thing a lane could hold, unlike a rendered comparison.

These are the names the shared sheets key rules off directly, read at tree fingerprint `29c5c0e71121027c`, and the list is a reading rather than a fact about the future.
The single-word ones are where collisions actually happen, because a hyphenated compound is already most of the way to being a prefix.

```
.account  .actions  .app      .auth     .avatar   .badge    .band     .banner
.btn      .cards    .chip     .chips    .choice   .conn     .counter  .cta
.disclosure .divider .dot     .drop     .field    .form     .grow     .impersonating
.link     .menu     .meter    .mono     .notice   .page     .panel    .picker
.placeholder .quiet .rail     .refusal  .refusals .region   .req      .row
.s        .search   .stat     .status   .tabbar   .tick     .toast    .toasts
.toggle   .top      .when     .wordmark
```

Forty-five hyphenated names are bare in the same sense and carry the same rule, among them `.head-row`, `.foot-note`, `.log-search`, `.page-head`, `.row-card`, `.tab-bar` and the nine `.seg-` tones.
They are listed here only as a caution, because a page author reaching for one of those has usually meant to.
