// The two calls this page makes that no other page makes: the marketplace
// request it posts, and the download manifest it reads.
//
// Beside the page rather than in `$lib/api`, which is the shared client every
// other route reads: nothing else in this console sends a marketplace request
// or reads a release manifest, so a shape here costs the shared module
// nothing.

import { ApiFailure, type APIErrorBody } from '$lib/api';

/** The bounds `crates/tam-api/src/marketplace_requests.rs` enforces, mirrored
 *  so the form can refuse before the round trip and count what the server
 *  counts. A bound changed there and not here costs a refusal the seller could
 *  have been shown a keystroke earlier; it never lets a bad value through,
 *  because the server checks again. */
export const NAME_MAX_CHARS = 120;
export const URL_MAX_CHARS = 2048;
export const REASON_MAX_CHARS = 2000;

/**
 * How many characters the server will count in this string.
 *
 * Rust counts `char`s, which are Unicode scalar values; `String.length` counts
 * UTF-16 code units, so anything above the basic plane — an emoji, a
 * mathematical letter — counts twice here and once there. A seller typing an
 * emoji into a 120-character field would be refused by a counter the server
 * disagrees with, so this counts the way the server does.
 */
export function charCount(text: string): number {
	return [...text].length;
}

/** Whether this is an address the server's own `is_web_address` accepts.
 *
 *  Deliberately the same looseness: a scheme test and a host test, not a
 *  parser. Anything stricter here would refuse a shop address the server would
 *  have taken, which costs a request we wanted. */
export function isWebAddress(url: string): boolean {
	const folded = url.toLowerCase();
	const scheme = ['https://', 'http://'].find((prefix) => folded.startsWith(prefix));
	if (scheme === undefined) {
		return false;
	}
	const rest = url.slice(scheme.length);
	const host = rest.split(/[/?#]/, 1)[0] ?? '';
	return host.length > 0 && !/\s/.test(host);
}

export interface MarketplaceRequestBody {
	name: string;
	url: string;
	reason: string;
}

/** One request as stored, which is the answer rather than the echo: every field
 *  is trimmed on the way in, so rendering what was sent would show values the
 *  row does not hold. */
export interface MarketplaceRequestView {
	id: string;
	org: string;
	requested_by: string;
	name: string;
	url: string;
	reason: string;
	created_at: number;
}

/**
 * Records where else this seller sells.
 *
 * Every refusal arrives as a 422 carrying one sentence written for the person
 * who typed the field — an address already asked about, more requests than one
 * organisation may hold, a field over its bound — so the form renders
 * `ApiFailure.message` rather than deciding what went wrong from the status.
 */
export async function sendMarketplaceRequest(
	body: MarketplaceRequestBody
): Promise<MarketplaceRequestView> {
	const response = await fetch('/v1/marketplace-requests', {
		method: 'POST',
		headers: { accept: 'application/json', 'content-type': 'application/json' },
		body: JSON.stringify(body)
	});
	if (!response.ok) {
		let parsed: APIErrorBody | null = null;
		try {
			parsed = (await response.json()) as APIErrorBody;
		} catch {
			parsed = null;
		}
		throw new ApiFailure(response.status, parsed);
	}
	return (await response.json()) as MarketplaceRequestView;
}

/** One published build: the file's name under `/downloads/`, the digest to check
 *  it against, and the release that file actually is.
 *
 *  `version` is per entry because the producer emits it per entry, and it does
 *  so deliberately: the Windows installer comes from an update channel while the
 *  Android package comes from its own listing, so the two sit at different
 *  releases routinely. `docs/notes/design/console-serving.md` states the rule it
 *  is protecting — "a manifest never reports one number for two different
 *  releases" — so an entry that carries its own version is the only number that
 *  may be printed beside that entry's file name. */
export interface DownloadEntry {
	file: string;
	sha256: string;
	/** Absent where the producer emits none, as it does for Windows, whose
	 *  release is the channel's. */
	version?: string;
}

/** What `/downloads/downloads.json` holds. A platform is `null` until a build
 *  for it is published, and the whole document is absent until the first one
 *  is, so both absences mean the same thing to the page. */
export interface DownloadsManifest {
	version: string;
	windows: DownloadEntry | null;
	android: DownloadEntry | null;
	apple: DownloadEntry | null;
}

/** A file name that stays inside `/downloads/`.
 *
 *  The manifest is written by our own release step and served from our own
 *  origin, so this is not a trust boundary today. It is a cheap one to hold
 *  anyway: the value is concatenated into an href, and a name carrying a slash,
 *  a colon or a leading dot would build a link somewhere other than the
 *  download it claims to be. */
const SAFE_FILE = /^[A-Za-z0-9][A-Za-z0-9._-]*$/;

function entry(value: unknown): DownloadEntry | null {
	if (typeof value !== 'object' || value === null) {
		return null;
	}
	const held = value as Record<string, unknown>;
	const file = held.file;
	const sha256 = held.sha256;
	if (typeof file !== 'string' || typeof sha256 !== 'string') {
		return null;
	}
	if (!SAFE_FILE.test(file)) {
		return null;
	}
	const version = held.version;
	return typeof version === 'string' && version.length > 0
		? { file, sha256, version }
		: { file, sha256 };
}

/**
 * Reads the manifest, answering `null` where there is nothing to offer.
 *
 * Absent, unreadable and malformed are one answer, because they are one fact
 * for the seller: no build is published, and every card reads "Coming soon".
 * A partial manifest keeps whatever platforms parse and drops the rest, so one
 * bad entry does not withdraw a download that exists.
 */
export function readManifest(document: unknown): DownloadsManifest | null {
	if (typeof document !== 'object' || document === null) {
		return null;
	}
	const held = document as Record<string, unknown>;
	if (typeof held.version !== 'string' || held.version.length === 0) {
		return null;
	}
	return {
		version: held.version,
		windows: entry(held.windows),
		android: entry(held.android),
		apple: entry(held.apple)
	};
}

/** Where a published build is fetched from. One place builds this string, so a
 *  card cannot link somewhere the manifest did not name. */
export function downloadHref(file: string): string {
	return `/downloads/${file}`;
}

export async function fetchManifest(): Promise<DownloadsManifest | null> {
	try {
		const response = await fetch('/downloads/downloads.json', {
			headers: { accept: 'application/json' }
		});
		if (!response.ok) {
			return null;
		}
		return readManifest(await response.json());
	} catch {
		return null;
	}
}
