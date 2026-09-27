import { defineConfig } from 'astro/config';

export default defineConfig({
	// The host serving this build; the canonical link and the Open Graph URL
	// are built from it.
	site: 'https://teachouse.io',
	output: 'static',
	// Never inlined: `tam-server` sends `style-src 'self'` for the landing and
	// hashes inline scripts only, so an inlined <style> (Astro's default for a
	// small sheet, which the maintenance page's is) would be blocked.
	build: { inlineStylesheets: 'never' },
	devToolbar: { enabled: false }
});
