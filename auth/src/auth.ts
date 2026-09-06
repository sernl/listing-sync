import { passkey } from '@better-auth/passkey';
import { type BetterAuthOptions, betterAuth } from 'better-auth';
import { createAuthMiddleware, getIP, isAPIError } from 'better-auth/api';
import { admin, captcha, haveIBeenPwned, jwt, openAPI } from 'better-auth/plugins';
import { PostgresDialect } from 'kysely';
import pg from 'pg';
import { type AuthEvent, record } from './audit.ts';
import { deliver } from './email.ts';
import { env } from './env.ts';
import { vouchedByProvider } from './provider-profile.ts';
import { greetingFor, utcTime } from './template.ts';

export const pool = new pg.Pool({ connectionString: env.databaseUrl });

const dialect = new PostgresDialect({ pool });

// Both providers map their profile through the same mapper, and the reason is
// in provider-profile.ts: without it a Microsoft address arrives unverified and
// the completion mail is never sent to it.
const socialProviders = {
  ...(env.google === undefined
    ? {}
    : { google: { ...env.google, mapProfileToUser: vouchedByProvider } }),
  ...(env.microsoft === undefined
    ? {}
    : { microsoft: { ...env.microsoft, mapProfileToUser: vouchedByProvider } }),
};

const audit = (event: AuthEvent): void => record(pool, event);

interface Origin {
  readonly ipAddress: string | undefined;
  readonly userAgent: string | undefined;
}

// better-auth's own resolver rather than reading x-forwarded-for directly: it
// honours the trusted-proxy and ipv6-subnet settings, so an audit row and the
// session row beside it agree on what the client's address was.
const origin = (headers: Headers | undefined, options: BetterAuthOptions): Origin => ({
  ipAddress: (headers === undefined ? null : getIP(headers, options)) ?? undefined,
  userAgent: headers?.get('user-agent') ?? undefined,
});

const submittedEmail = (body: unknown): string | undefined => {
  if (typeof body !== 'object' || body === null) {
    return undefined;
  }
  const email = (body as { email?: unknown }).email;
  return typeof email === 'string' ? email : undefined;
};

interface MintedSession {
  readonly session: { readonly id: string };
  readonly user: { readonly id: string };
}

type Subject = Pick<AuthEvent, 'userId' | 'sessionId' | 'identifier'>;

// The submitted address is recorded only while no user id exists to record
// instead; see the identifier note in audit.ts.
const subject = (minted: MintedSession | null, submitted: string | undefined): Subject =>
  minted === null
    ? { identifier: submitted }
    : { userId: minted.user.id, sessionId: minted.session.id };

// The admin plugin declares impersonatedBy on the session table
// (packages/better-auth/src/plugins/admin/schema.ts:29-37) and fills it with
// the acting admin's user id (plugins/admin/routes.ts:1283). A database hook
// receives Session & Record<string, unknown>, so a plugin's own column arrives
// untyped and is narrowed here rather than asserted.
const impersonatedBy = (session: Record<string, unknown>): string | undefined => {
  const actor = session['impersonatedBy'];
  return typeof actor === 'string' && actor.length > 0 ? actor : undefined;
};

// A rejected endpoint leaves its APIError on ctx.context.returned rather than
// skipping the after hooks (packages/better-auth/src/api/dispatch.ts:405-431),
// which is what makes a failed sign-in recordable at all.
interface Recipient {
  readonly email: string;
  readonly name: string | null;
}

const recipient = (value: unknown): Recipient | undefined => {
  if (typeof value !== 'object' || value === null) {
    return undefined;
  }
  const { email, name } = value as { email?: unknown; name?: unknown };
  return typeof email === 'string' && email.length > 0
    ? { email, name: typeof name === 'string' ? name : null }
    : undefined;
};

// /change-password answers with the user it changed
// (`dist/api/routes/update-user.mjs`), and dispatch puts that body on
// ctx.context.returned immediately before the after hooks run
// (`dist/api/dispatch.mjs:240-242`). /admin/set-user-password answers with
// `{status: true}` and names its subject only in the request body, which is why
// the two paths read different places for the same fact.
const changedUser = (returned: unknown): Recipient | undefined =>
  recipient(
    typeof returned === 'object' && returned !== null
      ? (returned as { user?: unknown }).user
      : undefined,
  );

const targetUserId = (body: unknown): string | undefined => {
  if (typeof body !== 'object' || body === null) {
    return undefined;
  }
  const id = (body as { userId?: unknown }).userId;
  return typeof id === 'string' && id.length > 0 ? id : undefined;
};

type Changer = 'the account holder' | 'a Teachouse administrator';

/**
 * Tell the account holder their password moved, whoever moved it.
 *
 * Three endpoints store a new password and better-auth's `onPasswordReset`
 * fires from exactly one of them, the token reset
 * (`dist/api/routes/password.mjs:172`). The other two are reached through the
 * after hook below. A seller who is told about the change they asked for and
 * not about the one an administrator made would be worse informed the more
 * serious the event.
 */
const passwordChanged = (to: string, name: string | null, changer: Changer): void => {
  const byAdministrator = changer === 'a Teachouse administrator';
  deliver({
    to,
    subject: 'Your Teachouse password was changed',
    greeting: greetingFor(name),
    lead: byAdministrator
      ? `Your Teachouse password was changed by a Teachouse administrator on ${utcTime(new Date())}.`
      : `Your Teachouse password was changed on ${utcTime(new Date())}.`,
    action: 'Request a new reset',
    url: `${env.baseUrl}/reset`,
    illustration: undefined,
    closing: byAdministrator
      ? 'If you did not expect this, use the button above to set a password only you know.'
      : 'If this was not you, use the button above to request a new reset straight away.',
  });
};

const failureCode = (returned: unknown): string | undefined => {
  if (!isAPIError(returned)) {
    return undefined;
  }
  const code = (returned.body as { code?: unknown } | undefined)?.code;
  return typeof code === 'string' ? code : String(returned.status);
};

export const auth = betterAuth({
  appName: 'tam-auth',
  baseURL: env.baseUrl,
  // The origins the browser is allowed to speak from, which better-auth checks
  // on every state-changing request and on every callbackURL it is handed. The
  // base URL is trusted implicitly; this is everything else, and in development
  // it is the whole reason a request from the vite dev server is not a CSRF
  // refusal.
  trustedOrigins: [...env.trustedOrigins],
  secret: env.secret,
  database: { dialect, type: 'postgres' },
  telemetry: { enabled: false },
  advanced: { database: { generateId: 'uuid' } },
  rateLimit: { enabled: true, storage: 'database' },
  // The session cookie cache is off, explicitly rather than by default.
  // Verified 2026-09-02 against better-auth 1.7.2: with it on, getSession
  // answers out of a signed cookie for `maxAge` seconds (default 300) without
  // reading the session table (`packages/better-auth/src/cookies/index.ts`),
  // so a session revoked through `revokeSession` stays usable until that cache
  // expires. Decision D14 puts a per-device sign-out on the "Your devices"
  // page, and a sign-out that silently takes up to five minutes is the one
  // thing that page must not do. Turning this on is therefore a founder
  // decision that has to answer for the revocation latency it reintroduces.
  session: { cookieCache: { enabled: false } },
  emailAndPassword: {
    enabled: true,
    // The password-changed notice tells the reader that resetting again evicts
    // whoever changed it. Without this better-auth skips deleteUserSessions
    // entirely (`dist/api/routes/password.mjs:173`) and, with the session
    // cookie cache off, a stolen session row stays valid through the reset the
    // email just asked for -- so the sentence would name a remedy the service
    // does not perform.
    revokeSessionsOnPasswordReset: true,
    sendResetPassword: async ({ user, url }) => {
      deliver({
        to: user.email,
        subject: 'Reset your Teachouse password',
        greeting: greetingFor(user.name),
        lead: 'You asked to reset the password on your Teachouse account. Choose a new one below and you will be straight back in.',
        action: 'Choose a new password',
        url,
        illustration: undefined,
        closing: 'If you did not request this, you can ignore this message.',
      });
    },
    // better-auth calls this after the new password is stored and before it
    // revokes sessions (`dist/api/routes/password.mjs`). It hands over the user
    // and no URL, so the button points at the console's request-a-reset page
    // rather than a tokenised link, which only `sendResetPassword` can mint.
    onPasswordReset: async ({ user }) => {
      deliver({
        to: user.email,
        subject: 'Your Teachouse password was changed',
        greeting: greetingFor(user.name),
        lead: `Your Teachouse password was changed on ${utcTime(new Date())}.`,
        action: 'Request a new reset',
        url: `${env.baseUrl}/reset`,
        illustration: undefined,
        closing: 'If this was not you, use the button above to request a new reset straight away.',
      });
    },
  },
  emailVerification: {
    sendOnSignUp: true,
    sendVerificationEmail: async ({ user, url }) => {
      deliver({
        to: user.email,
        subject: 'Welcome to Teachouse: confirm your email address',
        greeting: greetingFor(user.name),
        lead: 'Welcome to Teachouse. We keep your teaching resources in one place and list them on every marketplace you sell on, so you write a listing once instead of once per site.',
        action: 'Confirm your email address',
        url,
        illustration:
          'A courier with a satchel hands a book to someone at their front door, with New Zealand hills and ferns behind them.',
        closing: 'If you did not request this, you can ignore this message.',
      });
    },
  },
  account: {
    identityStrategy: 'provider-id',
    encryptOAuthTokens: true,
    accountLinking: { trustedProviders: ['google', 'microsoft'] },
  },
  socialProviders,
  hooks: {
    after: createAuthMiddleware(async (ctx) => {
      const where = origin(ctx.headers, ctx.context.options);
      const failed = failureCode(ctx.context.returned);
      const minted = ctx.context.newSession ?? null;
      const submitted = submittedEmail(ctx.body);

      if (ctx.path === '/sign-in/email') {
        if (failed !== undefined) {
          if (submitted !== undefined) {
            audit({ event: 'user_sign_in_failed', identifier: submitted, detail: failed, ...where });
          }
        } else if (minted !== null) {
          audit({ event: 'user_signed_in', ...subject(minted, submitted), ...where });
        }
        return;
      }

      // Social sign-in completes at the provider callback, not at
      // /sign-in/social, which only hands back the provider's URL. A minted
      // session is the whole condition: account linking runs through the same
      // path and mints none.
      if (ctx.path === '/callback/:id') {
        if (minted !== null) {
          audit({ event: 'user_signed_in', ...subject(minted, undefined), ...where });
        }
        return;
      }

      if (ctx.path === '/sign-up/email') {
        if (failed === undefined) {
          audit({ event: 'user_signed_up', ...subject(minted, submitted), ...where });
        }
        return;
      }

      if (ctx.path === '/request-password-reset') {
        if (failed === undefined && submitted !== undefined) {
          audit({ event: 'password_reset_requested', identifier: submitted, ...where });
        }
        return;
      }

      // POST /reset-password only. The GET /reset-password/:token beside it
      // redirects the link-click to the client and changes no password.
      if (ctx.path === '/reset-password' && failed === undefined) {
        audit({ event: 'password_reset_completed', ...where });
        return;
      }

      // The two password-mutation paths onPasswordReset does not reach. The
      // notice is sent from here rather than from a database hook because the
      // acting party is only distinguishable at the endpoint: both write the
      // same account row.
      if (ctx.path === '/change-password' && failed === undefined) {
        const changed = changedUser(ctx.context.returned);
        if (changed !== undefined) {
          passwordChanged(changed.email, changed.name, 'the account holder');
        }
        return;
      }

      if (ctx.path === '/admin/set-user-password' && failed === undefined) {
        const id = targetUserId(ctx.body);
        if (id === undefined) {
          return;
        }
        // The lookup is the only thing between a successful password set and
        // the notice, so a failing read must not turn a completed change into
        // a 500 the administrator would retry.
        try {
          const target = recipient(await ctx.context.internalAdapter.findUserById(id));
          if (target !== undefined) {
            passwordChanged(target.email, target.name, 'a Teachouse administrator');
          }
        } catch (cause: unknown) {
          console.error('tam-auth: could not notify a user of an administrative password set', cause);
        }
      }
    }),
  },
  databaseHooks: {
    session: {
      create: {
        // Impersonation is a session mint and has no other trace: the response
        // to /admin/impersonate-user carries the impersonated user but not the
        // admin, whose id reaches this row and nowhere else. Both parties are
        // readable here, which is the same reason the delete hook below exists.
        after: async (session, context) => {
          const actor = impersonatedBy(session);
          if (context === null || actor === undefined) {
            return;
          }
          audit({
            event: 'user_impersonated',
            userId: actor,
            targetUserId: session.userId,
            sessionId: session.id,
            ...origin(context.headers, context.context.options),
          });
        },
      },
      delete: {
        // The only seam that can attribute a sign-out: the endpoint deletes
        // the session row before any after hook runs and never puts it on the
        // response context, so this is where the subject is still readable.
        // The same holds for the end of an impersonation, which
        // /admin/stop-impersonating performs by deleting this row and answers
        // with the restored admin session, naming the impersonated user
        // nowhere.
        after: async (session, context) => {
          if (context === null) {
            return;
          }
          const where = origin(context.headers, context.context.options);
          const actor = impersonatedBy(session);

          // Discriminated on the row rather than on the path, so an
          // impersonation ended by signing out is recorded as what it was.
          // Deleting that session is not the impersonated user signing out,
          // and a row saying it was would be a false attribution in the one
          // table whose whole value is attribution.
          if (actor !== undefined) {
            audit({
              event: 'user_impersonation_stopped',
              userId: actor,
              targetUserId: session.userId,
              sessionId: session.id,
              ...where,
            });
            return;
          }

          if (context.path === '/sign-out') {
            audit({
              event: 'user_signed_out',
              userId: session.userId,
              sessionId: session.id,
              ...where,
            });
          }
        },
      },
    },
  },
  plugins: [
    jwt({
      jwks: { keyPairConfig: { alg: 'EdDSA', crv: 'Ed25519' } },
      jwt: {
        issuer: env.baseUrl,
        audience: 'tam-api',
        expirationTime: '2m',
        definePayload: ({ user }) => ({ email_verified: user.emailVerified }),
      },
    }),
    passkey({ rpID: env.passkeyRpId, rpName: env.passkeyRpName, origin: env.baseUrl }),
    admin(),
    haveIBeenPwned(),
    ...(env.turnstileSecretKey === undefined
      ? []
      : [captcha({ provider: 'cloudflare-turnstile', secretKey: env.turnstileSecretKey })]),
    ...(env.mode === 'development' ? [openAPI()] : []),
  ],
});
