// What the three download cards say, given whatever the release manifest holds.
// Pure, so it tests without a component.

import { type DownloadsManifest, downloadHref } from './api';
import type { Mark } from './catalogue';

export type Platform = 'windows' | 'android' | 'apple';

export const PLATFORM_ORDER: readonly Platform[] = ['windows', 'android', 'apple'];

/** What each card is headed.
 *
 *  The key is the manifest's, and the name is the reader's: `apple` is what the
 *  release manifest calls the entry and macOS is the only Apple platform a
 *  build could exist for, so a card headed "Apple" reads as an answer about
 *  iPhones to a founder asking about iPhones. There is no iOS build, no
 *  project and no developer account, so naming the platform is the honest
 *  heading.
 *
 *  The Android symbol is a licence condition rather than a flourish, and it is
 *  here rather than in the template because this string is the name's first
 *  appearance on the page. Google's brand guidelines, on the same page whose
 *  Creative Commons grant lets us draw the robot at all, say "Android™ should
 *  have a trademark symbol the first time it appears in a creative"; the
 *  second of Google's two name conditions, its trademark line, is already in
 *  `MARK_ATTRIBUTION`. `downloads.test.ts` fails if the symbol is tidied
 *  away. */
export const PLATFORM_NAME: Record<Platform, string> = {
	windows: 'Windows',
	android: 'Android™',
	apple: 'macOS'
};

/** What each download tile draws in its mark box.
 *
 *  No store badge on any of the three, whatever else they draw. Google, Apple
 *  and Microsoft each license their badge for one purpose, to link to a listing
 *  on that store, and none of these cards links to one: Windows and Android
 *  offer a file the release manifest names and a seller installs by hand, and no
 *  macOS build is published at all. A badge over a sideloaded file is outside
 *  the licence and tells the reader something untrue, which is a different thing
 *  from the guideline risk the founder accepted for marketplace logos on
 *  2026-09-05.
 *
 *  Past that the three owners do not permit alike, so the three tiles do not
 *  draw alike.
 *
 *  Android draws Google's own robot. Google's brand guidelines, at
 *  https://developer.android.com/distribute/marketing-tools/brand-guidelines,
 *  say "The green Android robot can be reproduced and/or modified as long as the
 *  following Creative Commons attribution line is included in the creative", and
 *  `MARK_ATTRIBUTION` carries that line on the page that draws it. The wordmark
 *  is refused by the same page and the name stays set in our own type.
 *
 *  Windows draws a glyph of ours. Microsoft's trademark guidelines say its
 *  logos, icons and designs "can never be used without an express license", and
 *  the Windows guidelines of February 2026 open with "A trademark use license is
 *  required to: Use any Windows logo, symbol or icon". The same document names
 *  an exception covering this exact row, and a request quoting it is drafted
 *  in `docs/notes/design/marketplace-logo-sources.md`; the founder decided on
 *  2026-09-07 not to send it, so the glyph is what stands.
 *
 *  macOS draws a glyph of ours and always will. Apple's marketing guidelines
 *  say "Don't use the standalone Apple logo", with no route to permission worth
 *  taking for a platform we publish no build for.
 *
 *  Typed `Mark` throughout, so a mark arriving with permission is a change to
 *  this table and to nothing else. */
export const PLATFORM_MARK: Record<Platform, Mark> = {
	windows: { kind: 'glyph', name: 'monitor' },
	android: { kind: 'image', shape: 'icon', src: '/vendors/android-robot.svg' },
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
	android:
		'It holds marketplace logins on the phone itself and runs your queued work when you open it, with no schedule of its own.',
	apple: 'The desktop app.'
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
