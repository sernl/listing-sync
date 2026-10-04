import { readdirSync } from 'node:fs';
import { basename } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { type DownloadsManifest, readManifest } from './api';
import {
	PLATFORM_MARK,
	PLATFORM_NAME,
	PLATFORM_ORDER,
	detectPlatform,
	downloadCards
} from './downloads';

/** The producer's own shape, from `nix/downloads-refresh.sh`: each platform at
 *  the release it last came from, so a skipped macOS job leaves an older Mac
 *  build beside a newer Windows one. Built through `readManifest` rather than
 *  written as a literal, so the fixture cannot drift into a shape the parser
 *  would never produce. */
const PUBLISHED = readManifest({
	schema: 2,
	version: '0.21.0',
	windows: {
		version: '0.21.0',
		updated: '2026-10-05T00:00:00Z',
		files: [
			{
				kind: 'exe',
				file: 'Teachouse_0.21.0_x64-setup.exe',
				sha256: 'a'.repeat(64),
				size: 9_400_000
			},
			{
				kind: 'msi',
				file: 'Teachouse_0.21.0_x64_en-US.msi',
				sha256: 'b'.repeat(64),
				size: 10_100_000
			}
		],
		store: 'https://apps.microsoft.com/detail/9ntest'
	},
	macos: {
		version: '0.20.0',
		updated: '2026-09-29T00:00:00Z',
		files: [
			{
				kind: 'dmg',
				file: 'Teachouse_0.20.0_universal.dmg',
				sha256: 'c'.repeat(64),
				size: 21_000_000
			}
		],
		store: null
	},
	linux: {
		version: '0.21.0',
		updated: '2026-10-05T00:00:00Z',
		files: [
			{
				kind: 'appimage',
				file: 'Teachouse_0.21.0_amd64.AppImage',
				sha256: 'd'.repeat(64),
				size: 80_000_000
			},
			{ kind: 'deb', file: 'Teachouse_0.21.0_amd64.deb', sha256: 'e'.repeat(64), size: 22_000_000 }
		],
		store: null
	},
	android: null,
	msstore_installers: ['Teachouse_0.21.0_x64_store-setup.exe'],
	refreshed_at: '2026-10-05T00:05:00Z'
}) as DownloadsManifest;

const card = (platform: string, device: Parameters<typeof downloadCards>[1] = null) =>
	downloadCards(PUBLISHED, device).find((held) => held.platform === platform);

describe('the download cards', () => {
	it('shows the four platforms in one order when the device is unknown', () => {
		expect(downloadCards(null).map((held) => held.platform)).toEqual(PLATFORM_ORDER);
		expect(downloadCards(PUBLISHED).map((held) => held.platform)).toEqual(PLATFORM_ORDER);
	});

	it("puts the reader's own device first and marks it, and only it", () => {
		const cards = downloadCards(PUBLISHED, 'macos');
		expect(cards.map((held) => held.platform)).toEqual(['macos', 'windows', 'linux', 'android']);
		expect(cards.filter((held) => held.current).map((held) => held.platform)).toEqual(['macos']);
	});

	it('offers nothing at all when no manifest is published', () => {
		expect(downloadCards(null).every((held) => held.offer === undefined)).toBe(true);
		expect(downloadCards(null).every((held) => held.store === undefined)).toBe(true);
	});

	it('draws a mark on every download card, published or not', () => {
		// A card with no build has no button either, so a missing mark would
		// leave it the emptiest thing on the screen. Both manifests, because the
		// mark comes from the platform and not from what the manifest named.
		for (const cards of [downloadCards(null), downloadCards(PUBLISHED)]) {
			for (const held of cards) {
				expect(held.mark, held.platform).toEqual(PLATFORM_MARK[held.platform]);
			}
		}
	});

	it('draws each platform the mark its owner permits, and no other', () => {
		// Google licenses the robot under Creative Commons with an attribution
		// line the page carries; Microsoft requires an express licence for the
		// Windows symbol, which we have not applied for; Apple forbids
		// third-party use of its logo outright. A change on any row is the moment
		// somebody has to have read a permission, and it fails here until they
		// have.
		expect(PLATFORM_MARK.android).toEqual({
			kind: 'image',
			shape: 'icon',
			src: '/vendors/android-robot.svg'
		});
		expect(PLATFORM_MARK.windows.kind).toBe('glyph');
		expect(PLATFORM_MARK.linux).toEqual({ kind: 'glyph', name: 'laptop' });
		expect(PLATFORM_MARK.macos.kind).toBe('glyph');
	});

	it("writes Android's name the way Google requires its trademark to be written", () => {
		// "Android™ should have a trademark symbol the first time it appears in a
		// creative", at
		// https://developer.android.com/distribute/marketing-tools/brand-guidelines.
		// This is the card heading, which is that first appearance.
		expect(PLATFORM_NAME.android).toBe('Android™');
		expect(downloadCards(null).map((held) => held.name)).toContain('Android™');
	});

	it('names a platform mark after the platform it stands for', () => {
		// Pointing Android at `/vendors/firefox.svg` passes every other assertion
		// and puts one vendor's logo on another vendor's card.
		for (const platform of PLATFORM_ORDER) {
			const mark = PLATFORM_MARK[platform];
			if (mark.kind === 'image') {
				expect(basename(mark.src), platform).toContain(platform);
			}
		}
	});

	it('ships no store badge in the directory a badge would arrive in', () => {
		// Store listings are linked in words. `web/static/` is served
		// unauthenticated, so a badge sitting there is published whether a card
		// draws it or not, and each badge programme licenses its artwork on
		// conditions of its own.
		const held = readdirSync(fileURLToPath(new URL('../../../../static/vendors', import.meta.url)));
		expect(held.filter((name) => /play|app-?store|microsoft-store|badge/i.test(name))).toEqual([]);
	});

	it('offers nothing for a platform whose entry is null', () => {
		const android = card('android');
		expect(android?.offer).toBeUndefined();
		expect(android?.store).toBeUndefined();
		expect(android?.body).toBe('Not available yet.');
	});

	it('links the main button to the first file the manifest named, and nowhere else', () => {
		expect(card('windows')?.offer).toEqual({
			label: 'Download for Windows',
			href: '/downloads/Teachouse_0.21.0_x64-setup.exe',
			file: 'Teachouse_0.21.0_x64-setup.exe',
			sha256: 'a'.repeat(64),
			size: '9 MB'
		});
	});

	it('offers the MSI and the .deb beside the main file', () => {
		expect(card('windows')?.alternatives.map((offer) => offer.href)).toEqual([
			'/downloads/Teachouse_0.21.0_x64_en-US.msi'
		]);
		expect(card('linux')?.offer?.href).toBe('/downloads/Teachouse_0.21.0_amd64.AppImage');
		expect(card('linux')?.alternatives.map((offer) => offer.label)).toEqual(['.deb package']);
	});

	it('offers the Mac disk image', () => {
		expect(card('macos')?.offer?.href).toBe('/downloads/Teachouse_0.20.0_universal.dmg');
		expect(card('macos')?.body).toContain('Apple silicon and Intel');
	});

	it('links a store listing in words where the manifest names one', () => {
		expect(card('windows')?.store).toEqual({
			label: 'Get it from the Microsoft Store',
			href: 'https://apps.microsoft.com/detail/9ntest'
		});
		expect(card('macos')?.store).toBeUndefined();
	});

	it('offers a store listing alone where no file is published', () => {
		const held = readManifest({
			version: '0.21.0',
			android: {
				version: null,
				updated: null,
				files: [],
				store: 'https://play.google.com/store/apps/details?id=io.teachouse.desktop'
			}
		});
		const android = downloadCards(held).find((entry) => entry.platform === 'android');
		expect(android?.offer).toBeUndefined();
		expect(android?.store?.label).toBe('Get it on Google Play');
		expect(android?.body).not.toBe('Not available yet.');
	});

	it('prints the release each platform actually is, not the newest one', () => {
		// The macOS job skipped on 0.21.0, so the Mac card offers 0.20.0 and must
		// say 0.20.0, beside a file whose name says the same.
		expect(card('macos')?.version).toBe('0.20.0');
		expect(card('windows')?.version).toBe('0.21.0');
	});

	it('never prints a version a file name contradicts', () => {
		for (const held of downloadCards(PUBLISHED)) {
			for (const offer of held.offer ? [held.offer, ...held.alternatives] : []) {
				expect(offer.href, held.platform).toContain(held.version);
			}
		}
	});

	it('carries the whole digest, which is what makes it worth printing', () => {
		expect(card('linux')?.alternatives[0]?.sha256).toHaveLength(64);
	});
});

describe('which device is reading', () => {
	it('reads the desktop platforms', () => {
		expect(detectPlatform({ platform: 'Win32' })).toBe('windows');
		expect(detectPlatform({ platform: 'MacIntel', maxTouchPoints: 0 })).toBe('macos');
		expect(detectPlatform({ userAgentData: { platform: 'macOS' } })).toBe('macos');
		expect(detectPlatform({ platform: 'Linux x86_64' })).toBe('linux');
	});

	it('reads Android from the user agent, which reports a Linux platform', () => {
		expect(
			detectPlatform({
				platform: 'Linux armv8l',
				userAgent: 'Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36'
			})
		).toBe('android');
	});

	it('offers a Chromebook the Android build', () => {
		expect(
			detectPlatform({
				platform: 'Linux x86_64',
				userAgent: 'Mozilla/5.0 (X11; CrOS x86_64 15633.69.0) AppleWebKit/537.36'
			})
		).toBe('android');
	});

	it('answers nothing for an iPhone or an iPad, for which there is no build', () => {
		expect(
			detectPlatform({ platform: 'iPhone', userAgent: 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0)' })
		).toBeNull();
		// iPadOS asks for the desktop site as a Mac; its touch points give it away.
		expect(detectPlatform({ platform: 'MacIntel', maxTouchPoints: 5 })).toBeNull();
		expect(detectPlatform({})).toBeNull();
	});
});
