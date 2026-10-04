import { createHash, timingSafeEqual } from 'node:crypto';
import { type IncomingMessage, type ServerResponse, createServer } from 'node:http';
import { toNodeHandler } from 'better-auth/node';
import { record } from './audit.ts';
import { auth, pool } from './auth.ts';
import { env } from './env.ts';
import { type Evidence, judge, parseProof } from './offboarding.ts';

const BASE_PATH = '/api/auth';
const ADDRESS_PATH = '/internal/address/';
const REAUTH_PATH = '/internal/reauth/';
const DELETE_PATH = '/internal/delete/';

const handler = toNodeHandler(auth);

const notFound = (response: import('node:http').ServerResponse): void => {
  response.writeHead(404, { 'content-type': 'application/json' });
  response.end('{"error":"tam-auth serves /api/auth/* only"}');
};

// Digested before comparing, because timingSafeEqual throws on a length
// mismatch and the length of a secret is itself a fact worth not leaking.
const secretMatches = (offered: string | undefined, configured: string): boolean => {
  if (offered === undefined) {
    return false;
  }
  return timingSafeEqual(
    createHash('sha256').update(offered).digest(),
    createHash('sha256').update(configured).digest(),
  );
};

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

// Schema-qualified for the reason audit.ts gives, and three columns because
// three is what the caller is owed: the address to send to, whether this
// service will vouch for it, and the name to greet with. Nothing about
// credentials, sessions or roles leaves here.
const ADDRESS = `SELECT "email", "emailVerified", "name" FROM auth."user" WHERE "id" = $1`;

/**
 * The address the platform holds for one subject, for the server that composes
 * a seller's completion mail.
 *
 * It exists only where TAM_AUTH_INTERNAL_SECRET is configured: with no secret
 * the path is an unknown path like any other and answers the same 404 it
 * always has, so a deployment that has not opted in has no route to reach.
 * Whether the reverse proxy in front of this service forwards /internal/* is a
 * separate question and a second fence; the secret is the one this service
 * owns.
 */
const address = async (
  subject: string,
  response: import('node:http').ServerResponse,
): Promise<void> => {
  if (!UUID.test(subject)) {
    notFound(response);
    return;
  }
  const found = await pool.query<{ email: string; emailVerified: boolean; name: string }>(
    ADDRESS,
    [subject],
  );
  const row = found.rows[0];
  if (row === undefined) {
    notFound(response);
    return;
  }
  response.writeHead(200, { 'content-type': 'application/json' });
  response.end(
    JSON.stringify({ email: row.email, emailVerified: row.emailVerified, name: row.name }),
  );
};

// The credential row's hash, where the account has one. better-auth names the
// email-and-password provider `credential`.
const PASSWORD_HASH = `SELECT "password" FROM auth."account"
    WHERE "userId" = $1 AND "providerId" = 'credential' AND "password" IS NOT NULL`;

// One statement: session, account and passkey rows reference the user
// `on delete cascade` (db/auth/0001_identity.sql).
const DELETE_USER = `DELETE FROM auth."user" WHERE "id" = $1`;

const json = (response: ServerResponse, status: number, body: unknown): void => {
  response.writeHead(status, { 'content-type': 'application/json' });
  response.end(JSON.stringify(body));
};

// The internal callers send a few hundred bytes; anything past this is not one
// of them.
const BODY_MAX_BYTES = 16 * 1024;

const readJson = async (request: IncomingMessage): Promise<unknown> => {
  const chunks: Buffer[] = [];
  let size = 0;
  for await (const chunk of request) {
    const buffer = chunk as Buffer;
    size += buffer.length;
    if (size > BODY_MAX_BYTES) {
      return undefined;
    }
    chunks.push(buffer);
  }
  try {
    return JSON.parse(Buffer.concat(chunks).toString('utf8')) as unknown;
  } catch {
    return undefined;
  }
};

/**
 * Whether the seller deleting their account is who the session says, for
 * tam-api's `DELETE /v1/account`. See offboarding.ts for the rule; this reads
 * the evidence it is judged on and answers the address on success.
 */
const reauth = async (
  subject: string,
  request: IncomingMessage,
  response: ServerResponse,
): Promise<void> => {
  const found = await pool.query<{ email: string; name: string }>(ADDRESS, [subject]);
  const user = found.rows[0];
  if (user === undefined) {
    notFound(response);
    return;
  }
  const proof = parseProof(await readJson(request));
  const hash = (await pool.query<{ password: string }>(PASSWORD_HASH, [subject])).rows[0]?.password;
  const passwordMatches =
    hash === undefined || proof.password === undefined
      ? undefined
      : await (await auth.$context).password.verify({ hash, password: proof.password });
  const read =
    proof.sessionCookie === undefined
      ? null
      : await auth.api
          .getSession({ headers: new Headers({ cookie: proof.sessionCookie }) })
          .catch(() => null);
  const evidence: Evidence = {
    hasPassword: hash !== undefined,
    passwordMatches,
    session:
      read === null
        ? undefined
        : { userId: read.user.id, createdAt: new Date(read.session.createdAt) },
  };
  const verdict = judge(subject, evidence, new Date());
  if (verdict.kind === 'refused') {
    json(response, verdict.status, { refusal: verdict.refusal });
    return;
  }
  json(response, 200, { email: user.email, name: user.name });
};

/** Deletes the identity account, and records that its holder did. */
const remove = async (subject: string, response: ServerResponse): Promise<void> => {
  const deleted = await pool.query(DELETE_USER, [subject]);
  if (deleted.rowCount === 0) {
    notFound(response);
    return;
  }
  record(pool, { event: 'user_removed', userId: subject, targetUserId: subject });
  response.writeHead(204);
  response.end();
};

const INTERNAL_ROUTES = [ADDRESS_PATH, REAUTH_PATH, DELETE_PATH] as const;

const server = createServer((request, response) => {
  const path = (request.url ?? '').split('?', 1)[0] ?? '';
  if (path === BASE_PATH || path.startsWith(`${BASE_PATH}/`)) {
    void handler(request, response);
    return;
  }
  const secret = env.internalSecret;
  const route = INTERNAL_ROUTES.find((prefix) => path.startsWith(prefix));
  if (secret !== undefined && route !== undefined) {
    const offered = request.headers['x-tam-internal-secret'];
    if (!secretMatches(typeof offered === 'string' ? offered : undefined, secret)) {
      // The same 404 an unconfigured deployment gives, so a caller without the
      // secret cannot tell a wrong secret from a route that is not there.
      notFound(response);
      return;
    }
    const subject = path.slice(route.length);
    if (route === ADDRESS_PATH) {
      void address(subject, response).catch(() => {
        json(response, 503, { error: 'the address could not be read' });
      });
      return;
    }
    // The two deletion routes change state, so they answer POST alone, and a
    // subject that is not a UUID is no subject this service holds.
    if (request.method !== 'POST' || !UUID.test(subject)) {
      notFound(response);
      return;
    }
    const work = route === REAUTH_PATH ? reauth(subject, request, response) : remove(subject, response);
    void work.catch((cause: unknown) => {
      console.error(`tam-auth: ${route} failed`, cause);
      if (!response.headersSent) {
        json(response, 503, { error: 'the account could not be read or deleted' });
      }
    });
    return;
  }
  notFound(response);
});

server.listen(env.port, env.bind, () => {
  console.log(`tam-auth listening on http://${env.bind}:${env.port}${BASE_PATH}`);
  console.log(`tam-auth base url ${env.baseUrl}, mode ${env.mode}`);
  if (env.internalSecret !== undefined) {
    console.log(
      `tam-auth answering ${INTERNAL_ROUTES.join(', ')} to a caller holding the shared secret`,
    );
  }
});
