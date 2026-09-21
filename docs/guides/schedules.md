---
topic: automations
tags: schedules, publishing, desktop-app
---

# Scheduling sends

A schedule sends resources to a marketplace at a time you choose.

## Before you start

1. Connect the marketplace.
2. Sign in on the desktop app and leave it running at the scheduled time.

A schedule with no connected marketplace, or no signed-in machine, waits instead of sending.

## Make a schedule

1. Open **Automations → Schedules**.
2. Choose the marketplace.
3. Choose what it sends: a label, a collection, or everything not yet sent.
4. Set the day, the time and the timezone.
5. Save.

<!-- shot: /automations/sharing, the schedule form with a time and timezone set -->
![Creating a schedule](/v1/guides/images/1049fc8e0d90df1b5f017419315ab894f4add6f4901fc57dfa3d507c93364d14)

## Timezone

The time you set is read in the timezone on the schedule, not in your browser's. Change the timezone on the schedule if you travel.

## What a schedule sends

Only resources that are ready: a target price, the marketplace's required fields answered, and a file on a signed-in machine. A resource that is not ready is skipped and listed, not failed.

## Republishing

A resource already live is only sent again when you have changed it. Turn on republishing if you want edits pushed on the next run.

## Tes cannot revise

Tes accepts a new listing but will not accept a revision to one already live. Take the listing down on Tes and let the schedule send it again.

<!-- shot: /automations/sharing, the schedule list showing the last run's result -->
![Schedule results](/v1/guides/images/73ac178bcbfb2dbf1e1d730c20f4f47a24bda1d7430c7abd45e5402b28deaeed)

## Check a run

1. Open **Automations → Schedules**.
2. Read the last run beside the schedule.
3. Open it to see each resource and what happened.
