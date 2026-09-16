import { defineConfig } from 'astro/config';

export default defineConfig({
	// The host serving this build; the canonical link and the Open Graph URL
	// are built from it.
	site: 'https://teachouse.io',
	output: 'static',
	devToolbar: { enabled: false }
});
