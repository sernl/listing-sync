// What the Export page does between the seller pressing the button and a file
// arriving on their computer: name the document, hand it to the host, and say
// what went wrong when nothing came back.
//
// The whole module is written against injected effects, the way `desktop.ts`
// takes an invoker, because the console's vitest environment is `node`: a
// function reaching for `document` or `fetch` on its own could not be tested
// at all, and the part worth testing here is the naming and the wording, not
// the anchor.

import { ApiFailure } from '$lib/api';

/** A CSV document as it came off the wire, already named. */
export interface CsvDocument {
	blob: Blob;
	filename: string;
}

/** Where the page stands.
 *
 * `working` is a state rather than a boolean because it is the one the seller
 * waits in, and a wait with no name renders as a button that did nothing.
 *
 * `handed` rather than `saved`, and the distinction is the point: see
 * [`saveDocument`]. This page can prove it gave the file to its host. It
 * cannot prove the host kept it, so it does not say so. */
export type ExportState =
	| { kind: 'idle' }
	| { kind: 'working' }
	| { kind: 'handed'; filename: string }
	| ({ kind: 'failed' } & Refusal);

/** What the file is called when the server did not say.
 *
 * Deliberately undated. The server names the file from its own UTC civil date
 * (`crates/tam-api/src/export.rs`), and a client that filled the gap with the
 * browser's date would put a different day on the file than every other copy
 * of the same export. No date is honest; the wrong date is not. */
export const FALLBACK_FILENAME = 'teachouse-resources.csv';

/** The pattern the page shows the seller, standing in for the day they ask. */
export const FILENAME_PATTERN = 'teachouse-resources-<date>.csv';

/** What the seller is told once the document has been handed over.
 *
 * Standing advice rather than an error arm, because whether the file was kept
 * is not observable from here. It is worded so that it is true both when the
 * host saved the file and when the host dropped it, and so that a seller who
 * finds nothing knows the next thing to try without having to be told a
 * failure this page cannot detect. */
export const HANDOFF_ADVICE =
	'Check your downloads. If nothing arrived, run the export in your browser.';

/** The name the server gave this document, read out of `Content-Disposition`.
 *
 * Readable at all only because the console and the API are one origin: the
 * desktop application loads the console from `control_plane::DEFAULT_BASE_URL`
 * and `api.ts` addresses the API by relative path, so no header is hidden
 * behind CORS exposure rules.
 *
 * The parameter is anchored at a boundary so that a parameter whose name
 * merely ends in `filename` cannot supply the value.
 *
 * `filename*`, the RFC 6266 extended form, is not read. This server emits only
 * the plain quoted form, and an unread extended parameter degrades to the
 * fallback name rather than to a wrong one.
 *
 * A name carrying a path separator is refused rather than cleaned, because
 * every name this server sends is `teachouse-resources-<date>.csv` and a
 * separator would mean something upstream is no longer the thing this parser
 * was written against. */
export function filenameFrom(disposition: string | null): string {
	if (disposition === null) {
		return FALLBACK_FILENAME;
	}
	const match = /(?:^|;)\s*filename\s*=\s*(?:"([^"]*)"|([^;]*))/i.exec(disposition);
	const raw = (match?.[1] ?? match?.[2] ?? '').trim();
	if (raw === '' || raw.includes('/') || raw.includes('\\')) {
		return FALLBACK_FILENAME;
	}
	return raw;
}

/** What the seller is told when a refusal arrived carrying no words of its own.
 *
 * `ApiFailure`'s constructor falls back to `request failed with <status>` when
 * the body is missing or unparseable, which is a sentence for whoever reads a
 * log and not for a seller. */
export const REFUSED_WITHOUT_REASON =
	'The export was refused and no reason came back. Try again in a moment.';

/** The one refusal whose remedy this page can offer a way to. */
export const SESSION_ENDED = 'Your session has ended. Sign in again, then export.';

/** Why no file arrived, and whether the page can offer the way out.
 *
 * `signIn` is carried rather than recovered by comparing the sentence, so the
 * wording and the remedy cannot drift apart. */
export interface Refusal {
	message: string;
	signIn: boolean;
}

/** What the seller is told when no file arrived.
 *
 * The export route itself answers only two ways — a 401 from the session
 * extractor and a 500 from a storage fault (`crates/tam-api/src/session.rs`,
 * `crates/tam-api/src/export.rs`) — and both are worded here rather than
 * rendered from the server's own sentence, because both of those are written
 * for whoever reads the log and neither names a remedy.
 *
 * Everything else in the 4xx range comes from between the client and that
 * route rather than from it: a proxy's 403 or 429 as HTML, or a 404 from a
 * control plane older than the console asking it, which
 * `web/src/lib/desktop.ts` documents as a state a seller can reach. Those
 * carry no parseable body, so the server's stated message is used only when
 * there actually is one. */
export function failureOf(caught: unknown): Refusal {
	if (caught instanceof ApiFailure) {
		if (caught.status === 401) {
			return { message: SESSION_ENDED, signIn: true };
		}
		if (caught.status >= 500) {
			return {
				message: 'The spreadsheet could not be built. Try again in a moment.',
				signIn: false
			};
		}
		const stated = caught.body?.errors?.[0]?.message;
		const message =
			typeof stated === 'string' && stated.trim() !== '' ? stated : REFUSED_WITHOUT_REASON;
		return { message, signIn: false };
	}
	return {
		message: 'The export could not be reached. Check your connection and try again.',
		signIn: false
	};
}

/** The sentence alone, for callers that render no remedy. */
export function failureMessage(caught: unknown): string {
	return failureOf(caught).message;
}

/** What the seller is told when handing the document over threw.
 *
 * Reachable only if the handover itself faults — a document that will not take
 * an element, or a host that refuses an object URL. It is not the case of a
 * host that accepts the click and quietly drops the file, which is not
 * observable; see [`saveDocument`]. */
export const HANDOFF_FAILED =
	'The spreadsheet was built, but this window would not take it. Try the export in your browser.';

/** Ask for the document and hand it to the host, reporting where that left the
 *  page.
 *
 * Returns the terminal state rather than setting one, so the caller owns its
 * own reactivity and this function stays testable without a component.
 *
 * The two failures are caught separately because they are not the same event
 * to the seller: one means no spreadsheet exists, the other means one does and
 * this window would not take it. Catching both in one arm would word a refused
 * handover as a connection fault. */
export async function runExport(
	request: () => Promise<CsvDocument>,
	save: (document: CsvDocument) => void
): Promise<ExportState> {
	let csv: CsvDocument;
	try {
		csv = await request();
	} catch (caught) {
		return { kind: 'failed', ...failureOf(caught) };
	}
	try {
		save(csv);
	} catch {
		return { kind: 'failed', message: HANDOFF_FAILED, signIn: false };
	}
	return { kind: 'handed', filename: csv.filename };
}

/** How long the object URL outlives the click that used it.
 *
 * Revoking in the same turn as the click races the download: the browser has
 * accepted the click but has not necessarily read the blob yet, and a revoked
 * URL cancels the transfer. The delay is generous rather than tuned, because
 * the only cost of holding the URL is the blob staying in memory until the
 * page is left, and the cost of revoking early is a file that never arrives. */
const REVOKE_DELAY_MS = 60_000;

/** Hand the document to whatever is hosting this console.
 *
 * An anchor carrying `download`, rather than a navigation to the export route,
 * for two reasons that both matter more in the desktop application than in a
 * browser: a navigation replaces the console — which in the application is the
 * whole window — and a navigation that fails renders the API's JSON error
 * document where the page used to be, so the seller reads a fault this module
 * would otherwise have worded for them.
 *
 * Whether the file is then written is not observable from here, and no caller
 * may report that it was. `HTMLElement.click()` dispatches the event and
 * returns; a host that writes the file and a host that discards it are the
 * same call with the same result. The desktop application is today the second
 * kind — it registers no download handler, so under WebKitGTK and WKWebView a
 * `blob:` download is dropped with no error reaching this page — which is why
 * the state this produces is `handed` and the page's words claim only that the
 * file was passed on. */
export function saveDocument(csv: CsvDocument): void {
	const url = URL.createObjectURL(csv.blob);
	const anchor = document.createElement('a');
	anchor.href = url;
	anchor.download = csv.filename;
	anchor.rel = 'noopener';
	anchor.style.display = 'none';
	document.body.append(anchor);
	anchor.click();
	anchor.remove();
	setTimeout(() => URL.revokeObjectURL(url), REVOKE_DELAY_MS);
}
