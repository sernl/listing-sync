import { randomUUID } from 'node:crypto';
import { APIError } from 'better-auth/api';

/**
 * No account exists without the person's agreement to the terms, to owning
 * what they publish, and to being 18 or older.
 *
 * The sign-up form sends the two boxes as `consent` in the body of
 * /sign-up/email; a social sign-up sends them as `additionalData.consent` on
 * /sign-in/social with `requestSignUp`, and the before hook carries them
 * through the provider redirect in the signed OAuth state. The account is
 * refused before it is created when they are missing, and once it exists the
 * agreement is written to tam-server's `POST /internal/consent`, into the
 * app's own database where the operators can read it. A write that fails
 * removes the account it was for: the sign-up fails closed.
 *
 * Everything here is free of the database pool auth.ts opens at import, so the
 * hooks are testable with fakes.
 */

export const CONSENT_REFUSAL = 'Please tick both boxes to continue.';
export const CONSENT_UNAVAILABLE =
  'We could not record your agreement, so your account was not created. Please try again in a minute.';

export interface Consent {
  readonly terms_privacy: true;
  readonly ip_ownership: true;
  readonly age_18: true;
  /** The Terms' effective date the form showed, `YYYY-MM-DD`. */
  readonly version: string;
}

const VERSION = /^\d{4}-\d{2}-\d{2}$/u;

const field = (value: unknown, name: string): unknown =>
  typeof value === 'object' && value !== null ? (value as Record<string, unknown>)[name] : undefined;

/** The agreement, when every box is ticked and the version is a date. */
export const parseConsent = (value: unknown): Consent | undefined => {
  const version = field(value, 'version');
  if (
    field(value, 'terms_privacy') !== true ||
    field(value, 'ip_ownership') !== true ||
    field(value, 'age_18') !== true ||
    typeof version !== 'string' ||
    !VERSION.test(version)
  ) {
    return undefined;
  }
  return { terms_privacy: true, ip_ownership: true, age_18: true, version };
};

/** The 422 a sign-up without both boxes ticked is answered with. */
export const consentRefusal = (): APIError =>
  new APIError('UNPROCESSABLE_ENTITY', { code: 'CONSENT_REQUIRED', message: CONSENT_REFUSAL });

/** The 503 a sign-up whose agreement could not be recorded is answered with. */
export const consentUnavailable = (): APIError =>
  new APIError('SERVICE_UNAVAILABLE', {
    code: 'CONSENT_UNAVAILABLE',
    message: CONSENT_UNAVAILABLE,
  });

/** Where the agreement is written, or nothing configured to write it. */
export interface ConsentSink {
  readonly url: string;
  readonly secret: string;
}

export interface ConsentRecord {
  readonly subject: string;
  readonly email: string | undefined;
  readonly ipAddress: string | undefined;
  readonly userAgent: string | undefined;
  readonly consent: Consent;
}

type Fetch = (input: string, init: RequestInit) => Promise<Response>;

/**
 * Writes one agreement to tam-server. Throws on anything but a 2xx, and when
 * no sink is configured, so the caller's failure path is the only path a
 * missing record can take.
 */
export const recordConsent = async (
  sink: ConsentSink | undefined,
  record: ConsentRecord,
  send: Fetch = fetch,
): Promise<void> => {
  if (sink === undefined) {
    throw new Error('TAM_AUTH_CONSENT_URL or TAM_AUTH_INTERNAL_SECRET is not set');
  }
  const answer = await send(sink.url, {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'x-tam-internal-secret': sink.secret },
    body: JSON.stringify({
      subject: record.subject,
      email: record.email ?? null,
      ip_address: record.ipAddress ?? null,
      user_agent: record.userAgent ?? null,
      consent: record.consent,
    }),
    signal: AbortSignal.timeout(5000),
  });
  if (!answer.ok) {
    throw new Error(`tam-server answered ${answer.status} to the consent record`);
  }
};

interface Pending {
  readonly consent: Consent;
  readonly email: string | undefined;
  readonly at: number;
}

/** How long a created account waits for its endpoint to finish. */
const PENDING_TTL_MS = 10 * 60 * 1000;

/**
 * The agreements of accounts created in this process whose endpoint has not
 * finished yet, keyed by the user id the create hook chose. The after hook
 * takes its entry back out; an entry nothing took (an endpoint that failed
 * after the row was written) ages out rather than accumulating.
 */
export class PendingConsents {
  readonly #entries = new Map<string, Pending>();
  readonly #now: () => number;

  constructor(now: () => number = Date.now) {
    this.#now = now;
  }

  stash(id: string, consent: Consent, email: string | undefined): void {
    const now = this.#now();
    for (const [key, entry] of this.#entries) {
      if (now - entry.at > PENDING_TTL_MS) {
        this.#entries.delete(key);
      }
    }
    this.#entries.set(id, { consent, email, at: now });
  }

  take(id: string): Pending | undefined {
    const entry = this.#entries.get(id);
    this.#entries.delete(id);
    return entry;
  }

  get size(): number {
    return this.#entries.size;
  }
}

/** The endpoints that create an account from a person's own request. */
export const SIGN_UP_PATHS: Readonly<Record<string, true>> = {
  '/sign-up/email': true,
  '/sign-in/social': true,
  '/callback/:id': true,
};

/** The slice of an endpoint context the hooks below read. */
export interface HookContext {
  readonly path: string | undefined;
  readonly body?: unknown;
}

export interface ConsentHooks {
  /**
   * The endpoint before hook: /sign-up/email without both boxes is a 422, and
   * so is a social sign-up that asked to create an account without them. A
   * social sign-up's agreement is handed to `carry`, which puts it in the
   * signed OAuth state the callback reads back.
   */
  readonly before: (ctx: HookContext) => Promise<void>;
  /**
   * The user create hook. On a sign-up path an account with no agreement is
   * not created (`false`); one with an agreement is given its id here, so the
   * after hook can find the agreement again. Anything else (an administrator
   * creating an account) passes untouched.
   */
  readonly createUser: (
    user: { readonly email?: unknown },
    ctx: HookContext | null,
  ) => Promise<false | { data: { id: string } } | undefined>;
  /**
   * The endpoint after hook's half: when `userId` was created by this request,
   * write its agreement. `recorded` and `not-created` both let the response
   * stand; `failed` means the account has been removed and the caller must
   * answer the failure.
   */
  readonly complete: (
    userId: string | undefined,
    where: { readonly ipAddress: string | undefined; readonly userAgent: string | undefined },
  ) => Promise<'recorded' | 'not-created' | 'failed'>;
}

export interface ConsentDeps {
  readonly sink: ConsentSink | undefined;
  readonly pending: PendingConsents;
  /** Attach the agreement to the OAuth state being generated. */
  readonly carry: (consent: Consent) => Promise<void>;
  /** The agreement the OAuth state carried back to the callback. */
  readonly carried: () => Promise<Consent | undefined>;
  readonly removeUser: (id: string) => Promise<void>;
  readonly send?: Fetch;
  readonly newId?: () => string;
  readonly log?: (message: string, cause: unknown) => void;
}

export const consentHooks = (deps: ConsentDeps): ConsentHooks => ({
  before: async (ctx) => {
    if (ctx.path === '/sign-up/email') {
      if (parseConsent(field(ctx.body, 'consent')) === undefined) {
        throw consentRefusal();
      }
      return;
    }
    if (ctx.path === '/sign-in/social' && field(ctx.body, 'requestSignUp') === true) {
      const consent = parseConsent(field(field(ctx.body, 'additionalData'), 'consent'));
      if (consent === undefined) {
        throw consentRefusal();
      }
      await deps.carry(consent);
    }
  },

  createUser: async (user, ctx) => {
    const path = ctx?.path;
    if (path === undefined || SIGN_UP_PATHS[path] !== true) {
      return undefined;
    }
    const consent =
      path === '/sign-up/email'
        ? parseConsent(field(ctx?.body, 'consent'))
        : path === '/sign-in/social'
          ? parseConsent(field(field(ctx?.body, 'additionalData'), 'consent'))
          : await deps.carried();
    if (consent === undefined) {
      return false;
    }
    const id = (deps.newId ?? randomUUID)();
    deps.pending.stash(id, consent, typeof user.email === 'string' ? user.email : undefined);
    return { data: { id } };
  },

  complete: async (userId, where) => {
    if (userId === undefined) {
      return 'not-created';
    }
    const pending = deps.pending.take(userId);
    if (pending === undefined) {
      return 'not-created';
    }
    try {
      await recordConsent(
        deps.sink,
        { subject: userId, email: pending.email, ...where, consent: pending.consent },
        deps.send,
      );
      return 'recorded';
    } catch (cause: unknown) {
      (deps.log ?? console.error)(
        'tam-auth: a sign-up agreement could not be recorded, so the account is being removed',
        cause,
      );
      try {
        await deps.removeUser(userId);
      } catch (removal: unknown) {
        (deps.log ?? console.error)(
          `tam-auth: account ${userId} has no agreement on record and could not be removed`,
          removal,
        );
      }
      return 'failed';
    }
  },
});
