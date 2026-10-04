---
topic: automations
tags: pricing, publishing
---

# Setting target prices

A price rule says what a resource should cost on the marketplace you send it to. Nothing changes until you approve the new prices.

**Automations → Pricing** has five steps: **Where**, **Rule**, **What**, **Preview** and **Approve**.

Price rules come with Pro and Studio. On Look and Starter you can still see and delete rules you saved before. See [Your plan and what it allows](/guides/plans).

## Step 1: Where

Choose where your resources come **From** and where they go **To**. The picture shows the rule that is on for that pair, with one of your resources as an example.

<!-- shot: /automations/pricing, step 1 Where from TPT to TES: × 0.79, up to the next .99, US$8.00 becomes £6.99 -->
![Prices from TPT to Tes, with an example](/v1/guides/images/103e108257a6b0842772e63ec6fd40b2476eb45640a5fca2d02151d95f6ebf90)

## Step 2: Rule

**Your price rules** lists the rules you have for this pair. Use **Edit**, **Duplicate** or **Delete** on a rule, or press **New price rule**.

**Suggestions** are ready-made rules. **Use this** fills in the form for you. Change it before you save: a suggestion is not advice about what your work is worth.

<!-- shot: /automations/pricing, step 2 Rule with one saved price rule and one suggestion -->
![Your price rules and a suggestion](/v1/guides/images/6ecd00076cab353fdf878f7b642e4ee738c1b48bb99a309b7e6c26ba9e92d094)

## Write a rule

1. Give it a **Name** and set it **On**.
2. Under **Multiply the price by**, type the rate. For TPT to Tes this turns dollars into pounds.
3. Under **Then round**, choose **Nearest penny** or **Up to the next .99**.
4. Open **Only some resources?** to cover only free or only paid resources, or resources with certain words in the description.
5. Press **Save this rule**.

<!-- shot: /automations/pricing, the rule form filled in from a suggestion: × 0.75, nearest penny, paid resources only -->
![A price rule started from a suggestion](/v1/guides/images/ce57861b18db4d38edf9b07b95031611cc469b607568f8d17cea6dd0fedaff9d)

## Currency and rounding

The rate is yours to set:

- **Use 0.75** fills in a flat estimate: 0.75 pounds for every dollar, until you change it.
- **Use today’s bank rate** fills in the European Central Bank's published daily rate and keeps it with the rule.

Rounding happens after the price is converted, so the price you see is the price that is sent. **Up to the next .99** never rounds below the converted amount: $5.00 at 0.79 is £3.95, which becomes £3.99.

## Steps 3 and 4: What and Preview

1. In **What**, choose **Every resource on** the marketplace, or **Let me choose** and tick some.
2. In **Preview**, choose **Rules that are on**, **Only rules I tick**, or **Only the rule I am writing**.
3. Press **Preview**.

**One-off changes for this preview** lets you try a different rate without changing the rule.

## Step 5: Approve

The table shows each resource's price **On TPT**, what it has **Now on TES**, and the **Proposed** price.

1. Tick the rows you want.
2. Press **Approve selected**, or **Approve all eligible** for every undecided row that is not blocked.
3. Press **Reject selected** or **Reject all undecided** for the ones you do not want.

<!-- shot: /automations/pricing, step 5 Approve with six proposed prices and four rows ticked -->
![Approving new prices](/v1/guides/images/06afff5a1f8138ed40dfeca05a326451a8ddd45c3b8e24f48a6d912a4e54b02b)

Nothing changes until you approve it. The same preview covers licences and resource types, so approving a row approves its terms too. See [Mapping your words to a marketplace's](/guides/target-terms).

## Apply without asking

In the rule form, open **Apply without asking?** and tick **Copy**, **Move** or **Cross-list**. The rule's price is then used for resources you copy, move or cross-list to that marketplace, without approving each one. With none ticked, the rule only proposes, and you approve prices in the preview.

## When a migration is held back by a missing price

Write a rule that covers that resource, approve its price, then preview the migration again. See [Moving a shop to another marketplace](/guides/migrations).
