---
topic: automations
tags: pricing, publishing
---

# Setting target prices

A target price rule sets what a resource should cost on a marketplace. Nothing changes until you approve the new prices.

## Write a rule

1. Open **Automations → Pricing**.
2. Choose the marketplace the rule is for.
3. Pick which resources it covers: a label, a collection, or all of them.
4. Choose how the price is worked out and save.

<!-- shot: /automations/pricing, the rule workbench with one saved rule -->
![A pricing rule](/v1/guides/images/f862a99c7cf93c09c4c8078b8d42bf9d7682ccf5257e7fabd690a13e1c1d4022)

## Presets are only a starting point

A preset fills in the rule for you. Edit it before you save. It is not advice about what your work is worth.

## Rounding

Set the rounding you want on the rule, for example to the nearest $0.25. Rounding happens after currency conversion, so the price you see is the price that is sent.

## Currency conversion

If the marketplace sells in another currency, your price is converted. Teachouse uses the exchange rate on the day the prices are worked out, and shows you that rate.

## Approve the prices

1. Open the list of new prices.
2. Check each resource's new price.
3. Approve the ones you want.

Nothing changes until you approve it.

<!-- shot: /automations/pricing, the plan with rows awaiting approval -->
![Approving new prices](/v1/guides/images/a2376c57b957698959fa7d6cecab58e0de63ab4312ba34d7d08dc1a5eb95a4d3)

## Apply automatically

Turn on **Apply automatically** to give new resources the rule's price without approving each one. Turn it off to approve prices yourself again.

## When a migration is held back by a missing price

Write a rule that covers that resource, approve its price, then run the migration again.
