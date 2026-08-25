import adapter from '@sveltejs/adapter-static';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

/** @type {import('@sveltejs/kit').Config} */
const config = {
  preprocess: vitePreprocess(),
  kit: {
    // A static single-page app served by tam-server's --ui-dir fallback;
    // unknown paths fall through to index.html for the client router.
    adapter: adapter({ fallback: 'index.html' })
  }
};

export default config;
