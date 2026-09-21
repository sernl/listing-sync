import adapter from '@sveltejs/adapter-static';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

/** @type {import('@sveltejs/kit').Config} */
const config = {
  preprocess: vitePreprocess(),
  kit: {
    // A static single-page app served by tam-server's --ui-dir fallback;
    // unknown paths fall through to index.html for the client router.
    adapter: adapter({ fallback: 'index.html' }),
    // Absolute asset URLs. The shell is served at every path by the
    // `--ui-dir` fallback, so a relative `../_app/…` resolves differently at
    // `/automations/schedules` than at `/`; session replay resolves the
    // build's own assets and needs the one form that is the same everywhere.
    paths: { relative: false }
  }
};

export default config;
