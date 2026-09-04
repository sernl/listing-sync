import { defineConfig } from 'astro/config';

export default defineConfig({
	// The host actually serving this build; it moves at the teachouse.io cutover.
	site: 'https://teachouse.stowiq.io',
	output: 'static',
	devToolbar: { enabled: false }
});
