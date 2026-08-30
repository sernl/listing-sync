import { passkey } from '@better-auth/passkey';
import { betterAuth } from 'better-auth';
import { admin, captcha, haveIBeenPwned, jwt, openAPI } from 'better-auth/plugins';
import { PostgresDialect } from 'kysely';
import pg from 'pg';
import { deliver } from './email.ts';
import { env } from './env.ts';

export const pool = new pg.Pool({ connectionString: env.databaseUrl });

const dialect = new PostgresDialect({ pool });

const socialProviders = {
  ...(env.google === undefined ? {} : { google: env.google }),
  ...(env.microsoft === undefined ? {} : { microsoft: env.microsoft }),
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
