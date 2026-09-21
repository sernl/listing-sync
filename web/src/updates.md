---
topic: automations
tags: connections, publishing, desktop-app
---

# Keeping marketplaces up to date

**Updates** does two things: it pulls what a marketplace holds, and it lists every change you have sent.

## Turn on pulling

1. Open **Automations → Updates**.
2. Choose the marketplace.
3. Set how often to pull and save.

A pull asks your computer to read each listing on the marketplace, so keep the desktop app signed in.

<!-- shot: /sync, the pull settings for one marketplace -->
__omp_shell("[Pull settings](image:updates-1)")

## Where pulled resources arrive

A pulled listing lands in **Catalogue → Resources**. One already in your catalogue is matched and updated rather than added twice.

## Published with catalogue words

A resource published from Teachouse carries your catalogue's wording, not the marketplace's. A pull will not overwrite your wording with theirs.

## Fills what the pull left empty

Where a pull finds no value, your template fills the gap. Fields the pull did find are left as the marketplace has them.

## Read a run

1. Open the run from the history list.
2. Read the outcome for each resource.
3. Open a resource to see each step we recorded.

Outcomes are kept apart from steps, so a failure tells you which step it stopped at.

<!-- shot: /sync/[id], one run's timeline with outcomes and steps -->
__omp_shell("[A run's timeline](image:updates-2)")

## When a change has more than one home

Where a change could apply to more than one marketplace, the run tells you which one it used. Set the rule you want under [Mapping your words to a marketplace's](/guides/target-terms).
