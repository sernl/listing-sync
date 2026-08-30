import { sveltekit } from '@sveltejs/kit/vite';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vitest/config';

export default defineConfig({
  plugins: [tailwindcss(), sveltekit()],
  server: {
    // The dev flow: Vite serves the client, tam-server serves the API, and
    // the proxy keeps them same-origin so the HttpOnly cookie and the
    // EventSource behave exactly as they will behind ServeDir.
    proxy: {
      '/v1': 'http://127.0.0.1:8080',
      '/healthz': 'http://127.0.0.1:8080',
      '/api/auth': 'http://127.0.0.1:8081'
    }
  },
  test: {
    include: ['src/**/*.test.ts'],
    environment: 'node'
  }
});
