---
title: Teachouse restore drill
---

# Teachouse restore drill

A backup that has never been restored is a hypothesis.
This is the experiment, and the operational charter's section 10 states the pass criteria it has to meet.
Run it once before there is customer data, then quarterly, and monthly for the first two quarters while the procedure is still wrong in ways nobody has found.
Record each run under `docs/ops/restore-drills/`, so a drill that did not happen is visible as a gap in a directory listing rather than invisible as an absence of memory.

## Before the first drill: break the circular dependency

restic encrypts with a repository password and safix decrypts with an age key.
If the only copies of either live on `derecho`, then there is no backup — only an archive nobody can open — and the drill's whole purpose is to prove that is not the case.

Hold offline, off `derecho`, and outside this repository:

- the restic repository password, the same value granted as `teachouse-backup/teachouse-restic-password`,
- the sops age key that opens the safix store,
- the entitlement signing key's private half,
- the blob key-encryption key, without which every sealed object is unrecoverable ciphertext.

The last two are not backup material in the ordinary sense and are named here anyway, because a restored database whose objects cannot be opened has restored a catalogue of nothing.

## The drill

This procedure assumes the deployment owns its Postgres cluster, which is what `services.teachouse.database.provision = "cluster"` means.
On a host where the cluster belongs to something else, the module refuses `backup.enable` for reasons it states in the assertion, the mechanism below is a nightly `pg_dump` from the host's own `services.postgresqlBackup` rather than point-in-time recovery, and steps 1 and 2 change accordingly — restore the dump, and accept that the recovery point is the age of the last one.

Provision a scratch NixOS virtual machine from the same flake, so the system under test is the system that runs.

1. Restore Postgres to a chosen timestamp: `pgbackrest restore --stanza=default --type=time --target='<timestamp>' --target-action=promote`, with `--set` to pin a specific backup.
2. Restore the object store from restic into the scratch machine's `/var/lib/teachouse/blobs`.
3. Boot the application: the migration oneshot runs first, then the three services.
4. Run the assertions below.

Start the clock at step 1 and stop it when assertion 4 passes.

## The pass criteria

1. The restored database's applied migration set, per `sqlx migrate info --source <migrations>/rust`, matches the set the restored binary embeds.
2. For a tenant chosen before the drill, the listing count, the most recent job-ledger row and the subscription state match the values recorded before the drill began.
3. Every object a `product_file` row references is present in the restored object store and its content hash matches the stored hash.
4. A synthetic sync job runs to completion on the restored system.
5. The wall-clock time from the start of the restore to assertion 4 passing is inside the stated recovery-time objective — one hour for Postgres, four for the object store — measured rather than estimated.

A failed drill is an incident with a written follow-up, and a skipped drill counts as a failed one.

## What the drill does not cover, and why it is worth saying

The drill restores data and proves the restored system serves.
It does not prove that the machine can be rebuilt from nothing, because provisioning a scratch virtual machine from the flake skips the step that would hurt most: a freshly provisioned box has no secrets until the age key decrypts them, and that key has to exist somewhere other than the box that died.
Exercising that path is the reason the offline material above is enumerated rather than assumed, and a drill that reaches for a key stored on `derecho` has proved the wrong thing.

## The deploy's own backup step

Independent of the schedule, the deploy task takes a fresh incremental backup immediately before running migrations.
The interval between the last scheduled backup and a destructive migration is exactly the interval you will wish were zero.
