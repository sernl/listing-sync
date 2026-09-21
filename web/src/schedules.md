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
__omp_shell("[Creating a schedule](image:schedules-1)")

## Timezone

The time you set is read in the timezone on the schedule, not in your browser's. Change the timezone on the schedule if you travel.

## What a schedule sends

Only resources that are ready: a target price, the marketplace's required fields answered, and a file on a signed-in machine. A resource that is not ready is skipped and listed, not failed.

## Republishing

A resource already live is only sent again when you have changed it. Turn on republishing if you want edits pushed on the next run.

## Tes cannot revise

Tes accepts a new listing but will not accept a revision to one already live. Take the listing down on Tes and let the schedule send it again.

<!-- shot: /automations/sharing, the schedule list showing the last run's result -->
__omp_shell("[Schedule results](image:schedules-2)")

## Check a run

1. Open **Automations → Schedules**.
2. Read the last run beside the schedule.
3. Open it to see each resource and what happened.
