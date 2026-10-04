/**
 * A seller deleting their own account, the identity service's half.
 *
 * tam-api runs the deletion (`crates/tam-api/src/account.rs`) and calls two
 * internal routes here, fenced by the shared secret the address route is:
 *
 * - `POST /internal/reauth/{subject}` checks the proof the seller offered.
 *   An account with a password must give it; any other account (Google,
 *   Microsoft, passkey only) must be signed in, in this browser, for less than
 *   `FRESH_SECONDS`. tam-api forwards this browser's session cookie for that.
 *   On success it answers the address and name, which the goodbye mail goes
 *   to before the next route forgets them.
 * - `POST /internal/delete/{subject}` deletes the user row. Sessions,
 *   linked accounts and passkeys reference it `on delete cascade`
 *   (db/auth/0001_identity.sql), so one statement takes them all, and every
 *   browser signed in to the account is signed out with it. The act is
 *   recorded as `user_removed` with the user as both parties: they removed
 *   themselves.
 *
 * The decision is a pure function so a test can hold it without a database.
 */

/** better-auth's own default `freshAge` is a day; deleting an account asks for
 *  a sign-in from the last five minutes instead. */
export const FRESH_SECONDS = 5 * 60;

/** What the seller offered, as tam-api forwards it. */
export interface Proof {
  readonly password: string | undefined;
  readonly sessionCookie: string | undefined;
}

/** The request body, narrowed. Anything malformed is an empty proof, which
 *  every account refuses. */
export const parseProof = (body: unknown): Proof => {
  if (typeof body !== 'object' || body === null) {
    return { password: undefined, sessionCookie: undefined };
  }
  const { password, sessionCookie } = body as { password?: unknown; sessionCookie?: unknown };
  return {
    password: typeof password === 'string' && password.length > 0 ? password : undefined,
    sessionCookie:
      typeof sessionCookie === 'string' && sessionCookie.length > 0 ? sessionCookie : undefined,
  };
};

/** What was found out about the account and the browser. */
export interface Evidence {
  /** Whether the account has a password (a `credential` account row). */
  readonly hasPassword: boolean;
  /** Whether the offered password matched; undefined when none was checked. */
  readonly passwordMatches: boolean | undefined;
  /** The user and age of the browser's session, where the cookie named one. */
  readonly session: { readonly userId: string; readonly createdAt: Date } | undefined;
}

export type Verdict =
  | { readonly kind: 'confirmed' }
  | { readonly kind: 'refused'; readonly status: 403 | 422; readonly refusal: Refusal };

export type Refusal = 'password' | 'password_required' | 'stale';

/**
 * Whether the proof stands. A password account is judged on its password
 * alone, so a seller who signed in with Google an hour ago but also set a
 * password is asked for it; an account with no password is judged on this
 * browser's sign-in, which must be the subject's own and fresh.
 */
export const judge = (subject: string, evidence: Evidence, now: Date): Verdict => {
  if (evidence.hasPassword) {
    if (evidence.passwordMatches === undefined) {
      return { kind: 'refused', status: 422, refusal: 'password_required' };
    }
    return evidence.passwordMatches
      ? { kind: 'confirmed' }
      : { kind: 'refused', status: 403, refusal: 'password' };
  }
  const session = evidence.session;
  if (
    session === undefined ||
    session.userId !== subject ||
    now.getTime() - session.createdAt.getTime() >= FRESH_SECONDS * 1000
  ) {
    return { kind: 'refused', status: 403, refusal: 'stale' };
  }
  return { kind: 'confirmed' };
};
