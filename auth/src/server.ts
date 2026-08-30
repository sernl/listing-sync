import { createServer } from 'node:http';
import { toNodeHandler } from 'better-auth/node';
import { auth } from './auth.ts';
import { env } from './env.ts';

const BASE_PATH = '/api/auth';

const handler = toNodeHandler(auth);

const server = createServer((request, response) => {
  const path = (request.url ?? '').split('?', 1)[0] ?? '';
  if (path === BASE_PATH || path.startsWith(`${BASE_PATH}/`)) {
    void handler(request, response);
    return;
  }
  response.writeHead(404, { 'content-type': 'application/json' });
  response.end('{"error":"tam-auth serves /api/auth/* only"}');
});

server.listen(env.port, env.bind, () => {
  console.log(`tam-auth listening on http://${env.bind}:${env.port}${BASE_PATH}`);
  console.log(`tam-auth base url ${env.baseUrl}, mode ${env.mode}`);
});
