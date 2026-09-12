# Operator markings: platform operator and identity admin

- date: 2026-09-12
- status: in use; both steps were run for the founder on thunderstorm on this date
- companions: `../design/admin-backoffice.md` for why the two markings are separate

## Two markings, two planes

An operator holds two markings that are granted separately and consulted separately, by the design in `admin-backoffice.md`.
The **platform operator** marking is a row in `platform_operator` on the application plane; it opens `/admin/*` on the API and the console.
The **identity admin** marking is `role = 'admin'` on `auth."user"` on the identity plane; it opens better-auth's admin plugin, which is what lists identity users, their sessions, signs a user out everywhere, bans, sets roles and impersonates.
Neither is derived from the other: a platform operator without the identity marking meets "not an identity admin" on `/admin/users`, which is the 403 the console reported until the second step below was run.

## Granting the platform operator marking

On the host, as the operator with database credentials, the `tam-admin` one-shot grants it; no HTTP path creates the first operator.

## Granting the identity admin marking

No command in this tree writes `auth."user".role`, because the application roles hold no privilege on that table by design, so the step is one statement as the database superuser on the host:

```sh
sudo -u postgres psql -d tam -c "UPDATE auth.\"user\" SET role = 'admin' WHERE email = '<address>';"
```

Revoke it by setting the role back to `'user'`.
The marking takes effect on the person's next request; no sign-out is needed.

## What each opens

| surface | needs |
|---|---|
| `/admin`, organisations, plans, health, failures, import drain, dead letters, guides | platform operator |
| `/admin/users` identity list, sessions, sign out everywhere, ban, role, impersonate | identity admin (and platform operator to reach the page) |
| `/admin/impersonations` (the trail) | platform operator; deliberately not the identity admin, so the party who impersonates is not the party who reads the trail |
