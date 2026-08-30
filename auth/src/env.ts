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
  readonly secret: string;
  readonly databaseUrl: string;
  readonly passkeyRpId: string;
  readonly passkeyRpName: string;
  readonly resendApiKey: string | undefined;
  readonly emailFrom: string | undefined;
  readonly turnstileSecretKey: string | undefined;
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
    secret: readRequired('BETTER_AUTH_SECRET'),
    databaseUrl: readRequired('TAM_AUTH_DATABASE_URL'),
    passkeyRpId: read('TAM_AUTH_PASSKEY_RP_ID') ?? baseUrl.hostname,
    passkeyRpName: read('TAM_AUTH_PASSKEY_RP_NAME') ?? 'Listing Sync',
    resendApiKey,
    emailFrom,
    turnstileSecretKey: readRequiredInProduction(mode, 'TURNSTILE_SECRET_KEY'),
    google: readOAuthPair('GOOGLE_CLIENT_ID', 'GOOGLE_CLIENT_SECRET'),
    microsoft:
      microsoft === undefined
        ? undefined
        : { ...microsoft, tenantId: read('MICROSOFT_TENANT_ID') ?? 'common' },
  };
};

export const env: Env = load();
