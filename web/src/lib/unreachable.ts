import { ApiFailure } from '$lib/api';

/** Why the console's first request did not get a Teachouse answer. */
export interface Unreachable {
	/** The HTTP status that came back, or null when nothing did. */
	status: number | null;
	/** What kind of failure it was, which decides the sentence. */
	kind: 'network' | 'edge' | 'server' | 'refused';
	/** One sentence for the seller, and the same one for a bug report. */
	sentence: string;
	/** What was actually thrown or answered, for the report the sentence
	 *  cannot carry: the error's own name and message, or the status. */
	detail: string;
}

/** The sentence for each outcome, decided by what came back rather than by
 *  what should have.
 *
 *  A failure with no structured body is not ours: this origin answers every
 *  route under `/v1` with JSON, so HTML or nothing on a 403 or a 5xx came from
 *  an edge in front of it — a bot check, a cache, a proxy — and the sentence
 *  says so, because "signing in again" does not clear a challenge the seller
 *  never saw. A network failure is `fetch` throwing rather than answering:
 *  no status at all, and the sentence names the device's own connection. A
 *  body that would not parse lands here too, as the SyntaxError it threw, so
 *  the detail line tells the two apart. */
export function describeUnreachable(failure: unknown): Unreachable {
	if (failure instanceof ApiFailure) {
		const ours = failure.body !== null;
		if (failure.status >= 500) {
			return {
				status: failure.status,
				kind: 'server',
				sentence: `Teachouse answered with an error (${failure.status}). Reloading is the first thing to try; if it keeps happening, tell us the time it happened.`,
				detail: `${failure.status} ${failure.message}`
			};
		}
		if (!ours) {
			return {
				status: failure.status,
				kind: 'edge',
				sentence: `Something between this device and Teachouse refused the request (${failure.status}) before it reached us. This is usually a network check that does not recognise the app; opening teachouse.stowiq.io in your browser once, then reopening the app, usually clears it.`,
				detail: `${failure.status} without a Teachouse body`
			};
		}
		return {
			status: failure.status,
			kind: 'refused',
			sentence: `Teachouse refused the request (${failure.status}): ${failure.message}`,
			detail: `${failure.status} ${failure.code() ?? ''}`.trim()
		};
	}
	return {
		status: null,
		kind: 'network',
		sentence:
			'This device could not reach Teachouse at all. Check its connection, then reload; if the connection is fine, a VPN or a private DNS setting on the device may be sending Teachouse somewhere it is not.',
		detail: failure instanceof Error ? `${failure.name}: ${failure.message}` : String(failure)
	};
}

/** What a second look finds once the screen is up, for the same report.
 *
 *  Run after the sentence is on screen rather than before, so a device that
 *  cannot reach anything is not made to wait on two more attempts before it
 *  is told. Each probe is one fact: whether the page itself came off the
 *  device's own cache (a page that did can render with no network at all,
 *  which is how a device shows a console and then fails its first request),
 *  whether the platform believes it is online, and what one plain request to
 *  the health route answers when asked to bypass every cache. */
export async function probeReachability(): Promise<string> {
	const facts: string[] = [];
	const navigation = performance.getEntriesByType('navigation')[0] as
		| PerformanceResourceTiming
		| undefined;
	if (navigation !== undefined) {
		facts.push(navigation.transferSize === 0 ? 'page from cache' : 'page from network');
	}
	facts.push(navigator.onLine ? 'online' : 'offline');
	try {
		const answer = await fetch('/healthz', { cache: 'no-store' });
		facts.push(`healthz ${answer.status}`);
	} catch (error) {
		facts.push(`healthz ${error instanceof Error ? `${error.name}: ${error.message}` : String(error)}`);
	}
	return facts.join(' · ');
}
