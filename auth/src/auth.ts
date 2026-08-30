import { passkey } from '@better-auth/passkey';
import { type BetterAuthOptions, betterAuth } from 'better-auth';
import { createAuthMiddleware, getIP, isAPIError } from 'better-auth/api';
import { admin, captcha, haveIBeenPwned, jwt, openAPI } from 'better-auth/plugins';
import { PostgresDialect } from 'kysely';
import pg from 'pg';
import { type AuthEvent, record } from './audit.ts';
import { deliver } from './email.ts';
import { env } from './env.ts';

export const pool = new pg.Pool({ connectionString: env.databaseUrl });

const dialect = new PostgresDialect({ pool });

const socialProviders = {
  ...(env.google === undefined ? {} : { google: env.google }),
  ...(env.microsoft === undefined ? {} : { microsoft: env.microsoft }),
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

// A rejected endpoint leaves its APIError on ctx.context.returned rather than
// skipping the after hooks (packages/better-auth/src/api/dispatch.ts:405-431),
// which is what makes a failed sign-in recordable at all.
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
  secret: env.secret,
  database: { dialect, type: 'postgres' },
  telemetry: { enabled: false },
  advanced: { database: { generateId: 'uuid' } },
  rateLimit: { enabled: true, storage: 'database' },
  emailAndPassword: {
    enabled: true,
    sendResetPassword: async ({ user, url }) => {
      deliver({
        to: user.email,
        subject: 'Reset your Listing Sync password',
        lead: 'Use this link to choose a new password.',
        url,
      });
    },
  },
  emailVerification: {
    sendOnSignUp: true,
    sendVerificationEmail: async ({ user, url }) => {
      deliver({
        to: user.email,
        subject: 'Verify your Listing Sync email address',
        lead: 'Use this link to confirm this address belongs to you.',
        url,
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
      }
    }),
  },
  databaseHooks: {
    session: {
      delete: {
        // The only seam that can attribute a sign-out: the endpoint deletes
        // the session row before any after hook runs and never puts it on the
        // response context, so this is where the subject is still readable.
        after: async (session, context) => {
          if (context?.path !== '/sign-out') {
            return;
          }
          audit({
            event: 'user_signed_out',
            userId: session.userId,
            sessionId: session.id,
            ...origin(context.headers, context.context.options),
          });
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
