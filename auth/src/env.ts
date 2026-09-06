export type Mode = 'development' | 'production';

export interface OAuthCredentials {
  readonly clientId: string;
  readonly clientSecret: string;
}

export interface Env {
  readonly mode: Mode;
  readonly bind: string;
  readonly port: number;
  readonly baseUrl: string;
  readonly trustedOrigins: readonly string[];
  readonly secret: string;
  readonly databaseUrl: string;
  readonly passkeyRpId: string;
  readonly passkeyRpName: string;
  readonly resendApiKey: string | undefined;
  readonly emailFrom: string | undefined;
  readonly turnstileSecretKey: string | undefined;
  /**
   * The shared secret the internal address route is fenced by. Absent, that
   * route does not exist: the path answers the same 404 every unknown path
   * does, so a deployment that has not opted in has nothing to reach.
   */
  readonly internalSecret: string | undefined;
  readonly google: OAuthCredentials | undefined;
  readonly microsoft: (OAuthCredentials & { readonly tenantId: string }) | undefined;
}

class ConfigurationError extends Error {}

const read = (name: string): string | undefined => {
  const value = process.env[name];
  return value === undefined || value.trim() === '' ? undefined : value;
};

const readRequired = (name: string): string => {
  const value = read(name);
  if (value === undefined) {
    throw new ConfigurationError(`tam-auth: ${name} is not set`);
  }
  return value;
};

const readRequiredInProduction = (mode: Mode, name: string): string | undefined => {
  const value = read(name);
  if (value === undefined && mode === 'production') {
    throw new ConfigurationError(`tam-auth: ${name} is required when TAM_AUTH_ENV is production`);
  }
  return value;
};

const readMode = (): Mode => {
  const value = read('TAM_AUTH_ENV') ?? 'production';
  if (value !== 'development' && value !== 'production') {
    throw new ConfigurationError(
      `tam-auth: TAM_AUTH_ENV must be development or production, got ${value}`,
    );
  }
  return value;
};

const readPort = (): number => {
  const value = read('TAM_AUTH_PORT') ?? '8081';
  const port = Number(value);
  if (!Number.isInteger(port) || port < 1 || port > 65535) {
    throw new ConfigurationError(`tam-auth: TAM_AUTH_PORT must be a port number, got ${value}`);
  }
  return port;
};

const readBaseUrl = (): URL => {
  const value = readRequired('TAM_AUTH_BASE_URL');
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    throw new ConfigurationError(`tam-auth: TAM_AUTH_BASE_URL must be an absolute URL, got ${value}`);
  }
  // The deployment shape is /api/auth/* proxied onto the dashboard's own
  // origin, so a base URL carrying a path has nowhere to take effect and is
  // rejected rather than silently dropped.
  if (url.pathname !== '/' || url.search !== '' || url.hash !== '') {
    throw new ConfigurationError(`tam-auth: TAM_AUTH_BASE_URL must be a bare origin, got ${value}`);
  }
  return url;
};

// The browser reaches /api/auth through the client dev server's proxy, so the
// Origin better-auth sees is vite's own, never this service's base URL. Both
// loopback spellings and both ports, because vite drifts to 5174 when 5173 is
// taken and the developer's Origin is whichever one they opened.
const DEVELOPMENT_ORIGINS: readonly string[] = [
  'http://localhost:5173',
  'http://localhost:5174',
  'http://127.0.0.1:5173',
  'http://127.0.0.1:5174',
];

// better-auth compares a pattern against the request's origin verbatim
// (packages/better-auth/src/auth/trusted-origins.ts), so an entry carrying a
// path or a trailing slash matches nothing and surfaces as a blank CSRF
// refusal at sign-in rather than as a misconfiguration. Rejecting it here
// makes it a startup error naming the entry instead. A wildcard pattern such
// as https://*.example.com survives this check unchanged, which is what lets
// better-auth's own wildcard matching still be reachable.
const readTrustedOrigins = (mode: Mode): readonly string[] => {
  // better-auth splits BETTER_AUTH_TRUSTED_ORIGINS on commas and appends it to
  // its own trust list without consulting this module at all
  // (packages/better-auth/src/context/helpers.ts, getTrustedOrigins), which
  // would put an origin past the validation below. Refused rather than merged,
  // so that every trusted origin arrives through one checked path. The
  // condition is exactly the one better-auth acts on: it ignores an empty
  // value, so an empty assignment grants nothing and is left alone.
  const upstream = process.env.BETTER_AUTH_TRUSTED_ORIGINS;
  if (upstream !== undefined && upstream !== '') {
    throw new ConfigurationError(
      'tam-auth: BETTER_AUTH_TRUSTED_ORIGINS is not supported, because better-auth would trust it unvalidated; ' +
        'set TAM_AUTH_TRUSTED_ORIGINS instead, which is the one knob this service checks at startup',
    );
  }
  const configured = (read('TAM_AUTH_TRUSTED_ORIGINS') ?? '')
    .split(',')
    .map((entry) => entry.trim())
    .filter((entry) => entry !== '');
  for (const entry of configured) {
    let origin: string;
    try {
      origin = new URL(entry).origin;
    } catch {
      throw new ConfigurationError(
        `tam-auth: TAM_AUTH_TRUSTED_ORIGINS entry ${entry} is not an absolute URL`,
      );
    }
    if (origin !== entry) {
      throw new ConfigurationError(
        `tam-auth: TAM_AUTH_TRUSTED_ORIGINS entry ${entry} must be a bare origin, with no path and no trailing slash`,
      );
    }
  }
  return mode === 'development' ? [...DEVELOPMENT_ORIGINS, ...configured] : configured;
};

// A provider configured with an id but no secret fails at the OAuth callback
// rather than at startup, so the pair is rejected here as a unit.
const readOAuthPair = (idName: string, secretName: string): OAuthCredentials | undefined => {
  const clientId = read(idName);
  const clientSecret = read(secretName);
  if (clientId === undefined && clientSecret === undefined) {
    return undefined;
  }
  if (clientId === undefined || clientSecret === undefined) {
    throw new ConfigurationError(`tam-auth: ${idName} and ${secretName} must be set together`);
  }
  return { clientId, clientSecret };
};

const load = (): Env => {
  const mode = readMode();
  const baseUrl = readBaseUrl();
  const resendApiKey = readRequiredInProduction(mode, 'RESEND_API_KEY');
  const emailFrom = read('TAM_AUTH_EMAIL_FROM');
  if (resendApiKey !== undefined && emailFrom === undefined) {
    throw new ConfigurationError('tam-auth: TAM_AUTH_EMAIL_FROM is required when RESEND_API_KEY is set');
  }
  const microsoft = readOAuthPair('MICROSOFT_CLIENT_ID', 'MICROSOFT_CLIENT_SECRET');
  return {
    mode,
    bind: read('TAM_AUTH_BIND') ?? '127.0.0.1',
    port: readPort(),
    baseUrl: baseUrl.origin,
    trustedOrigins: readTrustedOrigins(mode),
    secret: readRequired('BETTER_AUTH_SECRET'),
    databaseUrl: readRequired('TAM_AUTH_DATABASE_URL'),
    passkeyRpId: read('TAM_AUTH_PASSKEY_RP_ID') ?? baseUrl.hostname,
    passkeyRpName: read('TAM_AUTH_PASSKEY_RP_NAME') ?? 'Teachouse',
    resendApiKey,
    emailFrom,
    turnstileSecretKey: readRequiredInProduction(mode, 'TURNSTILE_SECRET_KEY'),
    internalSecret: read('TAM_AUTH_INTERNAL_SECRET'),
    google: readOAuthPair('GOOGLE_CLIENT_ID', 'GOOGLE_CLIENT_SECRET'),
    microsoft:
      microsoft === undefined
        ? undefined
        : { ...microsoft, tenantId: read('MICROSOFT_TENANT_ID') ?? 'common' },
  };
};

export const env: Env = load();
