import { createHash, timingSafeEqual } from 'node:crypto';
import { createServer } from 'node:http';
import { toNodeHandler } from 'better-auth/node';
import { auth, pool } from './auth.ts';
import { env } from './env.ts';

const BASE_PATH = '/api/auth';
const ADDRESS_PATH = '/internal/address/';

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

const server = createServer((request, response) => {
  const path = (request.url ?? '').split('?', 1)[0] ?? '';
  if (path === BASE_PATH || path.startsWith(`${BASE_PATH}/`)) {
    void handler(request, response);
    return;
  }
  const secret = env.internalSecret;
  if (secret !== undefined && path.startsWith(ADDRESS_PATH)) {
    const offered = request.headers['x-tam-internal-secret'];
    if (!secretMatches(typeof offered === 'string' ? offered : undefined, secret)) {
      // The same 404 an unconfigured deployment gives, so a caller without the
      // secret cannot tell a wrong secret from a route that is not there.
      notFound(response);
      return;
    }
    void address(path.slice(ADDRESS_PATH.length), response).catch(() => {
      response.writeHead(503, { 'content-type': 'application/json' });
      response.end('{"error":"the address could not be read"}');
    });
    return;
  }
  notFound(response);
});

server.listen(env.port, env.bind, () => {
  console.log(`tam-auth listening on http://${env.bind}:${env.port}${BASE_PATH}`);
  console.log(`tam-auth base url ${env.baseUrl}, mode ${env.mode}`);
  if (env.internalSecret !== undefined) {
    console.log(`tam-auth answering ${ADDRESS_PATH}{subject} to a caller holding the shared secret`);
  }
});
