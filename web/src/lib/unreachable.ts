import { ApiFailure } from '$lib/api';

/** Why the console's first request did not get a Teachouse answer. */
export interface Unreachable {
	/** The HTTP status that came back, or null when nothing did. */
	status: number | null;
	/** What kind of failure it was, which decides the sentence. */
	kind: 'network' | 'protocol' | 'server' | 'refused';
	/** One sentence for the seller, and the same one for a bug report. */
	sentence: string;
	/** What was actually thrown or answered, for the report the sentence
	 *  cannot carry: the error's own name and message, or the status. */
	detail: string;
}

/** Describe the observed response without guessing which intermediary sent it. */
export function describeUnreachable(failure: unknown): Unreachable {
	if (failure instanceof ApiFailure) {
		if (failure.response?.problem !== undefined || (failure.body === null && failure.status < 500)) {
			const response = failure.response;
			return {
				status: failure.status,
				kind: 'protocol',
				sentence: `Something went wrong loading this page (error ${failure.status}). Reload; if it happens again, send us the details below.`,
				detail: response === null
					? `${failure.status} without a Teachouse error body`
					: [
						`${failure.status} ${response.content_type ?? 'no content type'}`,
						response.requested_path,
						response.redirected ? `redirected to ${response.final_path ?? 'an unknown route'}` : null,
						response.problem
					].filter(Boolean).join(' · ')
			};
		}
		if (failure.status >= 500) {
			return {
				status: failure.status,
				kind: 'server',
				sentence: `Something went wrong on our side (error ${failure.status}). Reload; if it keeps happening, tell us when it happened.`,
				detail: `${failure.status} ${failure.message}`
			};
		}
		return {
			status: failure.status,
			kind: 'refused',
			sentence: `Teachouse could not do that (error ${failure.status}): ${failure.message}`,
			detail: `${failure.status} ${failure.code() ?? ''}`.trim()
		};
	}
	if (failure instanceof SyntaxError) {
		return {
			status: null,
			kind: 'protocol',
			sentence: 'Something went wrong loading this page. Reload; if it happens again, tell us the time and page.',
			detail: 'SyntaxError: response parsing failed'
		};
	}
	return {
		status: null,
		kind: 'network',
		sentence:
			'We could not reach Teachouse. Check your internet connection, then reload; if it keeps happening, tell us the time and page.',
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
