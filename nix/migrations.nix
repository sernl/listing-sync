# Both migration sets under one store path, so the deployment's single runner
# names one path instead of reaching into two source trees. The two are kept
# apart inside it because they are applied by different roles: `rust/` by the
# role that owns the database, `auth/` by the role that owns the auth schema.
{ runCommand }:
runCommand "teachouse-migrations" { } ''
  mkdir -p $out
  cp -r ${../crates/tam-storage/migrations} $out/rust
  cp -r ${../db/auth} $out/auth
''
