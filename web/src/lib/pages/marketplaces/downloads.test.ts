import { describe, expect, it } from 'vitest';
import { type DownloadsManifest, readManifest } from './api';
import { PLATFORM_ORDER, downloadCards } from './downloads';

/** The producer's own shape, from `nix/module.nix`: a channel version at the
 *  top, and an Android entry carrying its own version beside its digest because
 *  the Windows installer and the Android package sit at different releases
 *  routinely. Built through `readManifest` rather than written as a literal, so
 *  the fixture cannot drift into a shape the parser would never produce. */
const PUBLISHED = readManifest({
	version: '0.3.1',
	windows: { file: 'teachouse-0.3.1-x64-setup.exe', sha256: 'aa'.repeat(32) },
	android: {
		file: 'teachouse-0.2.0-arm64.apk',
		sha256: 'bb'.repeat(32),
		version: '0.2.0'
	},
	apple: null,
	refreshed_at: '2026-09-05T00:00:00Z'
}) as DownloadsManifest;

describe('the download cards', () => {
	it('shows the three platforms in one order whatever the manifest holds', () => {
		expect(downloadCards(null).map((card) => card.platform)).toEqual(PLATFORM_ORDER);
		expect(downloadCards(PUBLISHED).map((card) => card.platform)).toEqual(PLATFORM_ORDER);
	});

	it('offers nothing at all when no manifest is published', () => {
		expect(downloadCards(null).every((card) => card.offer === undefined)).toBe(true);
	});

	it('offers nothing for a platform whose entry is null', () => {
		const apple = downloadCards(PUBLISHED).find((card) => card.platform === 'apple');
		expect(apple?.offer).toBeUndefined();
		expect(apple?.body).toBe('Not available yet.');
	});

	it('links a published build to the file the manifest named, and nowhere else', () => {
		const windows = downloadCards(PUBLISHED).find((card) => card.platform === 'windows');
		expect(windows?.offer).toEqual({
			label: 'Download for Windows',
			href: '/downloads/teachouse-0.3.1-x64-setup.exe',
			version: '0.3.1',
			sha256: 'aa'.repeat(32)
		});
	});

	it('prints the release the file actually is, not the channel it was read beside', () => {
		// The defect this replaces: the Android card read "Version 0.3.1" over a
		// button linking teachouse-0.2.0-arm64.apk, so the number, the digest and
		// the file name described two different releases.
		const android = downloadCards(PUBLISHED).find((card) => card.platform === 'android');
		expect(android?.offer?.version).toBe('0.2.0');
		expect(android?.offer?.href).toContain('0.2.0');
		expect(android?.offer?.version).not.toBe(PUBLISHED.version);
	});

	it('falls back to the channel version only where the entry states none', () => {
		// Windows is that case in the producer's own output: its release is the
		// channel's, so it emits no version of its own.
		const windows = downloadCards(PUBLISHED).find((card) => card.platform === 'windows');
		expect(PUBLISHED.windows?.version).toBeUndefined();
		expect(windows?.offer?.version).toBe(PUBLISHED.version);
	});

	it('never prints a version the file name contradicts, on either platform', () => {
		for (const card of downloadCards(PUBLISHED)) {
			if (card.offer) {
				expect(card.offer.href, card.platform).toContain(card.offer.version);
			}
		}
	});

	it('carries the whole digest, which is what makes it worth printing', () => {
		const android = downloadCards(PUBLISHED).find((card) => card.platform === 'android');
		expect(android?.offer?.sha256).toHaveLength(64);
	});
});
