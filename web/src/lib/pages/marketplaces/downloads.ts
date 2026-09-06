// What the three download cards say, given whatever the release manifest holds.
// Pure, so it tests without a component.

import { type DownloadsManifest, downloadHref } from './api';
import type { Mark } from './catalogue';

export type Platform = 'windows' | 'android' | 'apple';

export const PLATFORM_ORDER: readonly Platform[] = ['windows', 'android', 'apple'];

export const PLATFORM_NAME: Record<Platform, string> = {
	windows: 'Windows',
	android: 'Android',
	apple: 'Apple'
};

/** What each download tile draws in its mark box.
 *
 *  A neutral glyph rather than the platform owner's store badge, and the reason
 *  is the badge licence rather than taste. Google, Apple and Microsoft each
 *  grant their badge for one purpose, to link to a listing on that store, and
 *  none of these three cards links to one: the Windows and Android builds are a
 *  file the release manifest names and installs by hand, and no macOS build is
 *  published at all. A badge on a card offering a sideloaded file is outside the
 *  licence and misleading to the reader, which is a different thing from the
 *  guideline risk the founder accepted for marketplace logos on 2026-09-05.
 *
 *  Typed `Mark` rather than a glyph name, so the day a store listing exists the
 *  badge lands as a change to this table and to nothing else. */
export const PLATFORM_MARK: Record<Platform, Mark> = {
	windows: { kind: 'glyph', name: 'monitor' },
	android: { kind: 'glyph', name: 'smartphone' },
	apple: { kind: 'glyph', name: 'laptop' }
};

/** What a card says while there is nothing to download.
 *
 *  One line for all three, because from the page's side they are one fact: the
 *  manifest names no build for that platform. Why there is none — no build
 *  exists, or one exists and this run could not verify it — is the producer's
 *  business and is not in the manifest, so the card does not guess at it. */
const NOTHING_YET = 'Not available yet.';

/** What each card says once a build is published. */
const OFFERED: Record<Platform, string> = {
	windows:
		'The desktop app: it keeps your marketplace logins on your own computer and runs the schedule there.',
	android: 'The same console on your phone.',
	apple: 'The desktop app for macOS.'
};

export interface DownloadCard {
	platform: Platform;
	name: string;
	mark: Mark;
	body: string;
	/** Absent where nothing is published, which is what makes the card read
	 *  "Coming soon" and carry no button. A card never links to a file the
	 *  manifest did not name, so a missing build is stated rather than
	 *  offered. */
	offer?: {
		label: string;
		href: string;
		version: string;
		sha256: string;
	};
}

/**
 * The three cards, in the order the page shows them.
 *
 * A null manifest and a null entry inside one are the same answer: no build is
 * published for that platform. The manifest is absent until the first release
 * is, so the page renders three "Coming soon" cards rather than an error, which
 * is what today's state actually is.
 */
export function downloadCards(manifest: DownloadsManifest | null): DownloadCard[] {
	return PLATFORM_ORDER.map((platform) => {
		const entry = manifest === null ? null : manifest[platform];
		if (manifest === null || entry === null) {
			return {
				platform,
				name: PLATFORM_NAME[platform],
				mark: PLATFORM_MARK[platform],
				body: NOTHING_YET
			};
		}
		return {
			platform,
			name: PLATFORM_NAME[platform],
			mark: PLATFORM_MARK[platform],
			body: OFFERED[platform],
			offer: {
				label: `Download for ${PLATFORM_NAME[platform]}`,
				href: downloadHref(entry.file),
				// The entry's own release where it states one, and the channel's
				// only where it does not. The other way round prints the Windows
				// number over the Android file name, which is the one thing the
				// producer emits a second version to prevent.
				version: entry.version ?? manifest.version,
				sha256: entry.sha256
			}
		};
	});
}
