import { readdirSync } from 'node:fs';
import { basename } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { type DownloadsManifest, readManifest } from './api';
import { PLATFORM_MARK, PLATFORM_NAME, PLATFORM_ORDER, downloadCards } from './downloads';

/** The producer's own shape, from `nix/module.nix`: a channel version at the
 *  top, and an Android entry carrying its own version beside its digest because
 *  the Windows installer and the Android package sit at different releases
 *  routinely. Built through `readManifest` rather than written as a literal, so
 *  the fixture cannot drift into a shape the parser would never produce. */
const PUBLISHED = readManifest({
	version: '0.3.1',
	windows: { file: 'teachouse-0.3.1-x64-setup.exe', sha256: 'aa'.repeat(32) },
	linux: { file: 'Teachouse_0.9.1_amd64.AppImage', sha256: 'c'.repeat(64), version: '0.9.1' },
	android: {
		file: 'teachouse-0.2.0-arm64.apk',
		sha256: 'bb'.repeat(32),
		version: '0.2.0'
	},
	apple: null,
	refreshed_at: '2026-09-05T00:00:00Z'
}) as DownloadsManifest;

describe('the download cards', () => {
	it('shows the four platforms in one order whatever the manifest holds', () => {
		expect(downloadCards(null).map((card) => card.platform)).toEqual(PLATFORM_ORDER);
		expect(downloadCards(PUBLISHED).map((card) => card.platform)).toEqual(PLATFORM_ORDER);
	});

	it('offers nothing at all when no manifest is published', () => {
		expect(downloadCards(null).every((card) => card.offer === undefined)).toBe(true);
	});

	it('draws a mark on every download card, published or not', () => {
		// Before this, the three download tiles were the only cards on the page
		// with an empty caption where every other card carries a mark, and the
		// state that showed it was the unpublished one: a card with no build has
		// no button either, so a missing mark left it the emptiest thing on the
		// screen. Both manifests, because the mark comes from the platform and
		// not from what the manifest happened to name.
		for (const cards of [downloadCards(null), downloadCards(PUBLISHED)]) {
			for (const card of cards) {
				expect(card.mark, card.platform).toEqual(PLATFORM_MARK[card.platform]);
			}
		}
	});

	it('draws each platform the mark its owner permits, and no other', () => {
		// Three owners, three answers, so one assertion over all three would say
		// less than it looks like it says. Google licenses the robot under
		// Creative Commons with an attribution line the page carries; Microsoft
		// requires an express licence for the Windows symbol, which we have not
		// applied for; Apple forbids third-party use of its logo outright. A
		// change on any row is the moment somebody has to have read a permission,
		// and it fails here until they have.
		expect(PLATFORM_MARK.android).toEqual({
			kind: 'image',
			shape: 'icon',
			src: '/vendors/android-robot.svg'
		});
		expect(PLATFORM_MARK.windows.kind).toBe('glyph');
		expect(PLATFORM_MARK.linux).toEqual({ kind: 'glyph', name: 'laptop' });
		expect(PLATFORM_MARK.apple.kind).toBe('glyph');
	});

	it("writes Android's name the way Google requires its trademark to be written", () => {
		// "Android™ should have a trademark symbol the first time it appears in a
		// creative", at
		// https://developer.android.com/distribute/marketing-tools/brand-guidelines --
		// two lines above the attribution line the same page requires, and on the
		// same page whose Creative Commons grant is why the robot may be drawn at
		// all. This is the card heading, which is that first appearance. Dropping
		// the symbol is a one-character edit that reads as a tidy-up and leaves the
		// page carrying the second of Google's two name conditions and not the
		// first, which is the shape `catalogue.test.ts` already refuses for Chrome.
		expect(PLATFORM_NAME.android).toBe('Android™');
		expect(downloadCards(null).map((card) => card.name)).toContain('Android™');
	});

	it('names a platform mark after the platform it stands for', () => {
		// The mutation this closes: pointing Android at `/vendors/firefox.svg`
		// passes every other assertion here and on the catalogue side -- the file
		// exists, it is under `/vendors/`, it is an image -- and puts one
		// vendor's logo on another vendor's card, which is the single failure a
		// page carrying five owners' marks can least afford.
		for (const platform of PLATFORM_ORDER) {
			const mark = PLATFORM_MARK[platform];
			if (mark.kind === 'image') {
				expect(basename(mark.src), platform).toContain(platform);
			}
		}
	});

	it('ships no store badge in the directory a badge would arrive in', () => {
		// All three badge programmes license the badge for one purpose, to link
		// to a listing on that store, and none of these cards links to one. The
		// mark table above refuses the badge on the card; this refuses the file,
		// because `web/static/` is served unauthenticated and a badge sitting
		// there is published whether a card draws it or not.
		const held = readdirSync(fileURLToPath(new URL('../../../../static/vendors', import.meta.url)));
		expect(held.filter((name) => /play|app-?store|microsoft-store|badge/i.test(name))).toEqual([]);
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

	it('links the Linux card to the AppImage the manifest named', () => {
		const linux = downloadCards(PUBLISHED).find((card) => card.platform === 'linux');
		expect(linux?.offer).toEqual({
			label: 'Download for Linux',
			href: '/downloads/Teachouse_0.9.1_amd64.AppImage',
			version: '0.9.1',
			sha256: 'c'.repeat(64)
		});
		expect(linux?.body).toContain('.deb');
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
