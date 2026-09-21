---
topic: automations
tags: pricing, publishing
---

# Setting target prices

A target price rule says what a resource should cost on a marketplace. Nothing is sent until you approve the plan.

## Write a rule

1. Open **Automations → Pricing**.
2. Choose the marketplace the rule is for.
3. Pick which resources it covers, by label, collection or all.
4. Set how the price is worked out and save.

<!-- shot: /automations/pricing, the rule workbench with one saved rule -->
__omp_shell("[A pricing rule](image:target-prices-1)")

## Presets are suggestions

A preset fills the rule with a starting point. Edit it before you save; it is not advice about what your work is worth.

## Rounding

Set the rounding you want on the rule, for example to the nearest $0.25. Rounding is applied after the rate, so the price you see in the plan is the price that is sent.

## Where the rate is applied

A currency rate is applied when the marketplace sells in another currency. The rate used is the one on the day the plan is built, and the plan shows it.

## Approve the plan

1. Open the plan.
2. Read each row's new price.
3. Approve the rows you want.

Nothing applies until you approve it.

<!-- shot: /automations/pricing, the plan with rows awaiting approval -->
__omp_shell("[Approving priced rows](image:target-prices-2)")

## Apply automatically

Turn on **Apply automatically** to let new resources take the rule without you approving each one. Turn it off to keep approving by hand.

## When a migration says a row is blocked on price

Write a rule that covers that resource, approve the plan, then run the migration again.
