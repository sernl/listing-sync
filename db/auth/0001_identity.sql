-- better-auth's identity tables: platform user, browser session, credential
-- accounts, verification tokens, the JWT signing key set, passkeys, and the
-- rate-limit counters.
--
-- Generated from auth/src/auth.ts by `just auth-ddl`, then reviewed and landed
-- here. better-auth's own `migrate` is never pointed at a live database: the
-- workspace's migration discipline is a reviewed, numbered, ordered sequence,
-- and an opaque CLI mutating schema is at odds with it.
--
-- The statements below are verbatim generator output. They are not reformatted,
-- so re-running the generator against a changed config produces a diff that is
-- exactly the schema change and nothing else.
--
-- The Kysely adapter emits unqualified table names, so search_path alone decides
-- where these land. Applying as tam_auth is already enough, because the role
-- carries `search_path = auth` (db/init/02-auth-role.sql); the SET below states
-- the placement in the file rather than leaving it to the applying role.
--
-- Two columns hold secret material and neither is readable outside this schema:
-- account.password is the password hash, and jwks.privateKey is the JWT signing
-- key encrypted under BETTER_AUTH_SECRET. account.accessToken, refreshToken and
-- idToken are third-party OAuth tokens, encrypted under the same secret because
-- the service sets account.encryptOAuthTokens.
--
-- user.id is a uuid rather than better-auth's default 32-character string, so
-- app_user.auth_subject (migration 0035) joins on a uuid. This is fixed before
-- the first user row exists: changing it afterwards is a data migration across
-- every table here.

SET search_path = auth;

create table "user" ("id" uuid default pg_catalog.gen_random_uuid() not null primary key, "name" text not null, "email" text not null unique, "emailVerified" boolean not null, "image" text, "createdAt" timestamptz default CURRENT_TIMESTAMP not null, "updatedAt" timestamptz default CURRENT_TIMESTAMP not null, "role" text, "banned" boolean, "banReason" text, "banExpires" timestamptz);

create table "session" ("id" uuid default pg_catalog.gen_random_uuid() not null primary key, "expiresAt" timestamptz not null, "token" text not null unique, "createdAt" timestamptz default CURRENT_TIMESTAMP not null, "updatedAt" timestamptz not null, "ipAddress" text, "userAgent" text, "userId" uuid not null references "user" ("id") on delete cascade, "impersonatedBy" text);

create table "account" ("id" uuid default pg_catalog.gen_random_uuid() not null primary key, "issuer" text not null, "accountId" text not null, "providerId" text not null, "userId" uuid not null references "user" ("id") on delete cascade, "accessToken" text, "refreshToken" text, "idToken" text, "accessTokenExpiresAt" timestamptz, "refreshTokenExpiresAt" timestamptz, "scope" text, "password" text, "createdAt" timestamptz default CURRENT_TIMESTAMP not null, "updatedAt" timestamptz not null);

create table "verification" ("id" uuid default pg_catalog.gen_random_uuid() not null primary key, "identifier" text not null, "value" text not null, "expiresAt" timestamptz not null, "createdAt" timestamptz default CURRENT_TIMESTAMP not null, "updatedAt" timestamptz default CURRENT_TIMESTAMP not null);

create table "jwks" ("id" uuid default pg_catalog.gen_random_uuid() not null primary key, "publicKey" text not null, "privateKey" text not null, "createdAt" timestamptz not null, "expiresAt" timestamptz, "alg" text, "crv" text);

create table "passkey" ("id" uuid default pg_catalog.gen_random_uuid() not null primary key, "name" text, "publicKey" text not null, "userId" uuid not null references "user" ("id") on delete cascade, "credentialID" text not null, "counter" integer not null, "deviceType" text not null, "backedUp" boolean not null, "transports" text, "createdAt" timestamptz, "aaguid" text);

create table "rateLimit" ("id" uuid default pg_catalog.gen_random_uuid() not null primary key, "key" text not null unique, "count" integer not null, "lastRequest" bigint not null);

create index "session_userId_idx" on "session" ("userId");

create index "account_userId_idx" on "account" ("userId");

create index "verification_identifier_idx" on "verification" ("identifier");

create index "passkey_userId_idx" on "passkey" ("userId");

create index "passkey_credentialID_idx" on "passkey" ("credentialID");

create unique index "account_issuer_accountId_uidx" on "account" ("issuer", "accountId");
