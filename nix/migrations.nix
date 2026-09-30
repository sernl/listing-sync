# Both migration sets under one store path, so the deployment's single runner
# names one path instead of reaching into two source trees. The two are kept
# apart inside it because they are applied by different roles: `rust/` by the
# role that owns the database, `auth/` by the role that owns the auth schema.
#
# `guides/` is the help corpus the same step publishes after the schema: the
# Markdown `tam-admin guides seed` reads and, under `guides/images/`, the
# pictures `tam-admin guides images` places. It rides here because it is data
# the deploy writes into the database, like the migrations, and a runner that
# already names this path needs no second one.
{ runCommand }:
runCommand "teachouse-migrations" { } ''
  mkdir -p $out
  cp -r ${../crates/tam-storage/migrations} $out/rust
  cp -r ${../db/auth} $out/auth
  cp -r ${../docs/guides} $out/guides
''
