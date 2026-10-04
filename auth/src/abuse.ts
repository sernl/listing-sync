import { APIError } from 'better-auth/api';
import { SIGN_UP_PATHS } from './consent.ts';

/**
 * A sign-up is screened by tam-server before its account exists: a throwaway
 * email domain, too many accounts from one network in a day, or an identity a
 * banned account left behind is refused with the sentence tam-server chose.
 *
 * Unlike the agreement (consent.ts), the screen fails open. It guards against
 * one person farming free moves through many accounts, which is a cost, not a
 * harm a single wrongly admitted account does; refusing every teacher because
 * tam-server was unreachable would be the worse failure. So anything but a
 * clear 204 or 422 lets the sign-up through and leaves one line in the log.
 */

export const DISPOSABLE_EMAIL_REFUSAL = 'Please use a school or personal email address.';
export const IP_VELOCITY_REFUSAL =
  'Too many accounts were made from this network today. Please try again tomorrow or email contact@teachouse.io.';
export const SUSPENDED_REFUSAL = 'This account is suspended. Email contact@teachouse.io.';

const SCREEN_PATH = '/internal/abuse/screen';
const CONSENT_PATH = '/internal/consent';

/**
 * Where the screen lives when `TAM_AUTH_ABUSE_SCREEN_URL` is not set: beside
 * the consent endpoint, on the same tam-server.
 */
export const defaultScreenUrl = (consentUrl: string | undefined): string | undefined => {
  if (consentUrl === undefined) {
    return undefined;
  }
  if (consentUrl.includes(CONSENT_PATH)) {
    return consentUrl.replace(CONSENT_PATH, SCREEN_PATH);
  }
  try {
    return new URL(SCREEN_PATH, consentUrl).href;
  } catch {
    return undefined;
  }
};

/** Where sign-ups are screened, or nothing configured to screen them. */
export interface ScreenSink {
  readonly url: string;
  readonly secret: string;
}

type Fetch = (input: string, init: RequestInit) => Promise<Response>;

export interface AbuseDeps {
  readonly sink: ScreenSink | undefined;
  readonly send?: Fetch;
  readonly log?: (message: string, cause: unknown) => void;
}

/** The 422 a refused sign-up is answered with. */
export const signUpRefusal = (message: string): APIError =>
  new APIError('UNPROCESSABLE_ENTITY', { code: 'SIGNUP_REFUSED', message });

/** The sentence tam-server served with its refusal, when it can be read. */
const servedMessage = async (answer: Response): Promise<string | undefined> => {
  let body: unknown;
  try {
    body = await answer.json();
  } catch {
    return undefined;
  }
  if (typeof body !== 'object' || body === null || !('errors' in body)) {
    return undefined;
  }
  const first: unknown = Array.isArray(body.errors) ? body.errors[0] : undefined;
  if (typeof first !== 'object' || first === null || !('message' in first)) {
    return undefined;
  }
  return typeof first.message === 'string' && first.message.trim() !== ''
    ? first.message
    : undefined;
};

/**
 * Asks tam-server whether this sign-up may go ahead. Returns when it may, or
 * when the answer could not be had; throws the 422 when it was refused.
 */
export const screenSignUp = async (
  deps: AbuseDeps,
  who: { readonly email: string; readonly ipAddress: string | undefined },
): Promise<void> => {
  const log = deps.log ?? console.error;
  if (deps.sink === undefined) {
    log(
      'tam-auth: a sign-up was not screened because TAM_AUTH_ABUSE_SCREEN_URL or TAM_AUTH_INTERNAL_SECRET is not set',
      undefined,
    );
    return;
  }
  let answer: Response;
  try {
    answer = await (deps.send ?? fetch)(deps.sink.url, {
      method: 'POST',
      headers: { 'content-type': 'application/json', 'x-tam-internal-secret': deps.sink.secret },
      body: JSON.stringify({ email: who.email, ip_address: who.ipAddress ?? null }),
      signal: AbortSignal.timeout(5000),
    });
  } catch (cause: unknown) {
    log('tam-auth: a sign-up was let through because the abuse screen could not be reached', cause);
    return;
  }
  if (answer.status === 204) {
    return;
  }
  if (answer.status === 422) {
    throw signUpRefusal((await servedMessage(answer)) ?? DISPOSABLE_EMAIL_REFUSAL);
  }
  log(
    `tam-auth: a sign-up was let through because the abuse screen answered ${answer.status}`,
    undefined,
  );
};

/**
 * The user create hook's half: on a sign-up path the account is screened
 * before it is written, so a refusal means no row. An account an
 * administrator creates is not a sign-up and is not screened.
 */
export const screenNewUser = async (
  deps: AbuseDeps,
  user: { readonly email?: unknown },
  ctx: { readonly path: string | undefined; readonly ipAddress: string | undefined } | null,
): Promise<void> => {
  const path = ctx?.path;
  if (path === undefined || SIGN_UP_PATHS[path] !== true || typeof user.email !== 'string') {
    return;
  }
  await screenSignUp(deps, { email: user.email, ipAddress: ctx?.ipAddress });
};
