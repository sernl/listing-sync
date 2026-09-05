// What the three download cards say, given whatever the release manifest holds.
// Pure, so it tests without a component.

import { type DownloadsManifest, downloadHref } from './api';

export type Platform = 'windows' | 'android' | 'apple';

export const PLATFORM_ORDER: readonly Platform[] = ['windows', 'android', 'apple'];

export const PLATFORM_NAME: Record<Platform, string> = {
	windows: 'Windows',
	android: 'Android',
	apple: 'Apple'
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
			return { platform, name: PLATFORM_NAME[platform], body: NOTHING_YET };
		}
		return {
			platform,
			name: PLATFORM_NAME[platform],
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
