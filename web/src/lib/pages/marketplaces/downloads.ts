// What the download cards say, given whatever the release manifest holds and
// whichever device is reading. Pure, so it tests without a component.

import {
	type DownloadEntry,
	type DownloadFile,
	type DownloadKind,
	type DownloadPlatform,
	type DownloadsManifest,
	downloadHref
} from './api';
import type { Mark } from './catalogue';

export type Platform = DownloadPlatform;

export const PLATFORM_ORDER: readonly Platform[] = ['windows', 'macos', 'linux', 'android'];

/** What each card is headed.
 *
 *  The key is the manifest's, and the name is the reader's. macOS is the only
 *  Apple platform a build exists for — there is no iOS project — so naming the
 *  platform rather than the company is the honest heading.
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
	macos: 'macOS',
	linux: 'Linux',
	android: 'Android™'
};

/** What each download tile draws in its mark box.
 *
 *  No store badge on any of them, whatever else they draw. Where a platform has
 *  a store listing the card links it in words, beside the file: each badge
 *  programme licenses its artwork on conditions of its own, and words carry the
 *  same link without taking any of them on.
 *
 *  Past that the owners do not permit alike, so the tiles do not draw alike.
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
 *  say "Don't use the standalone Apple logo".
 *
 *  Linux draws a glyph of ours too. Tux is licensed for attribution-free use,
 *  but the landing draws only marks with a vendor policy on file and this
 *  page follows it: a glyph says the same thing and asks nothing of anyone.
 *
 *  Typed `Mark` throughout, so a mark arriving with permission is a change to
 *  this table and to nothing else. */
export const PLATFORM_MARK: Record<Platform, Mark> = {
	windows: { kind: 'glyph', name: 'monitor' },
	macos: { kind: 'glyph', name: 'laptop' },
	linux: { kind: 'glyph', name: 'laptop' },
	android: { kind: 'image', shape: 'icon', src: '/vendors/android-robot.svg' }
};

/** What a card says while there is nothing to download.
 *
 *  One line for all of them, because from the page's side they are one fact:
 *  the manifest names no build for that platform. Why there is none — no build
 *  exists, or one exists and the mirror could not verify it — is the
 *  producer's business and is not in the manifest, so the card does not guess
 *  at it. */
const NOTHING_YET = 'Not available yet.';

/** What each card says once a build is published. */
const OFFERED: Record<Platform, string> = {
	windows:
		'The desktop app. It keeps your marketplace logins on your own computer and runs your automations there. The MSI is for computers a school or IT team manages.',
	macos:
		'The desktop app, for Apple silicon and Intel Macs. It keeps your marketplace logins on your own computer and runs your automations there.',
	linux: 'AppImage for 64-bit Linux, or a .deb package for Debian and Ubuntu.',
	android:
		'Keeps your marketplace logins on your phone. Waiting work runs when you open the app, not on a schedule.'
};

/** How each file type is offered when it is not the card's main button. */
const KIND_LABEL: Record<DownloadKind, string> = {
	exe: 'Installer (.exe)',
	msi: 'MSI installer',
	dmg: 'Disk image (.dmg)',
	appimage: 'AppImage',
	deb: '.deb package',
	apk: 'Android package (.apk)'
};

/** The words each platform's store link carries. */
const STORE_LABEL: Record<Platform, string> = {
	windows: 'Get it from the Microsoft Store',
	macos: 'Get it from the Mac App Store',
	linux: '',
	android: 'Get it on Google Play'
};

export interface DownloadOffer {
	label: string;
	href: string;
	file: string;
	sha256: string;
	/** "72 MB", or absent where the producer measured nothing. */
	size?: string;
}

export interface DownloadCard {
	platform: Platform;
	name: string;
	mark: Mark;
	body: string;
	/** True for the card matching the device reading the page, which is drawn
	 *  first. */
	current: boolean;
	/** The release these files are, printed beside them. Absent with no file. */
	version?: string;
	/** The file the main button offers. Absent where nothing is published, which
	 *  is what makes the card read "Coming soon" and carry no button: a card
	 *  never links to a file the manifest did not name. */
	offer?: DownloadOffer;
	/** The platform's other files, such as the MSI or the .deb. */
	alternatives: DownloadOffer[];
	store?: { label: string; href: string };
}

function offer(file: DownloadFile, label: string): DownloadOffer {
	const held: DownloadOffer = {
		label,
		href: downloadHref(file.file),
		file: file.file,
		sha256: file.sha256
	};
	if (file.size !== null) {
		held.size = `${Math.max(1, Math.round(file.size / 1_000_000))} MB`;
	}
	return held;
}

/**
 * The device reading the page, as a platform this page offers a build for, or
 * null where it is one we have nothing for (an iPhone, an iPad) or cannot tell.
 *
 * `userAgentData.platform` first where the browser has it, then the older
 * `navigator.platform`. Android is read from the user agent before either,
 * because an Android phone reports a Linux `navigator.platform`. An iPad asking
 * for the desktop site reports `MacIntel`, and its touch points are what tell
 * it from a Mac. A Chromebook installs Android apps, so it is offered the
 * Android build.
 */
export function detectPlatform(device: {
	userAgent?: string;
	platform?: string;
	userAgentData?: { platform?: string };
	maxTouchPoints?: number;
}): Platform | null {
	const agent = device.userAgent ?? '';
	if (/Android/i.test(agent)) {
		return 'android';
	}
	if (/iPhone|iPad|iPod/i.test(agent)) {
		return null;
	}
	const platform = device.userAgentData?.platform || device.platform || '';
	if (/^Win/i.test(platform)) {
		return 'windows';
	}
	if (/^Mac/i.test(platform)) {
		return (device.maxTouchPoints ?? 0) > 1 ? null : 'macos';
	}
	if (/CrOS|Chrome OS/i.test(platform) || /CrOS/.test(agent)) {
		return 'android';
	}
	if (/Linux/i.test(platform)) {
		return 'linux';
	}
	return null;
}

function card(platform: Platform, entry: DownloadEntry | null, current: boolean): DownloadCard {
	const base = {
		platform,
		name: PLATFORM_NAME[platform],
		mark: PLATFORM_MARK[platform],
		current
	};
	const store =
		entry?.store != null ? { label: STORE_LABEL[platform], href: entry.store } : undefined;
	const [first, ...rest] = entry?.files ?? [];
	if (entry === null || entry.version === null || first === undefined) {
		return store === undefined
			? { ...base, body: NOTHING_YET, alternatives: [] }
			: { ...base, body: OFFERED[platform], alternatives: [], store };
	}
	return {
		...base,
		body: OFFERED[platform],
		version: entry.version,
		offer: offer(first, `Download for ${PLATFORM_NAME[platform]}`),
		alternatives: rest.map((file) => offer(file, KIND_LABEL[file.kind])),
		...(store === undefined ? {} : { store })
	};
}

/**
 * The cards, in the order the page shows them: the reader's own device first
 * where it is one we build for, then the rest in `PLATFORM_ORDER`.
 *
 * A null manifest and a null entry inside one are the same answer: nothing is
 * published for that platform. The manifest is absent until the first release
 * is, so the page then renders every card as "Coming soon" rather than an
 * error.
 */
export function downloadCards(
	manifest: DownloadsManifest | null,
	device: Platform | null = null
): DownloadCard[] {
	const order =
		device === null ? PLATFORM_ORDER : [device, ...PLATFORM_ORDER.filter((p) => p !== device)];
	return order.map((platform) =>
		card(platform, manifest === null ? null : manifest[platform], platform === device)
	);
}
