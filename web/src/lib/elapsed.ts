// How long ago an instant was, in the words the console uses everywhere it
// shows one. Pure, so it tests without a component.

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/** The age of an instant, rounded down so nothing ever reads fresher than it
 *  is. A reading ahead of `now` is clock skew rather than an instant from the
 *  future, and reads as just now rather than as a negative age. */
export function agoLabel(instant: number, now: number): string {
	const elapsed = Math.max(0, now - instant);
	if (elapsed < MINUTE) {
		return 'just now';
	}
	if (elapsed < HOUR) {
		return `${Math.floor(elapsed / MINUTE)} min ago`;
	}
	if (elapsed < DAY) {
		return `${Math.floor(elapsed / HOUR)} h ago`;
	}
	const days = Math.floor(elapsed / DAY);
	return `${days} ${days === 1 ? 'day' : 'days'} ago`;
}

/** An instant written out in full, in UTC.
 *
 * The console shows ages rather than instants nearly everywhere; this is for
 * the operator surface, where an exact instant is the point. The trailing `Z`
 * is not decoration: without it the reading is indistinguishable from a local
 * time, which is the one thing it is not. */
export function utcInstant(instant: number): string {
	return `${new Date(instant).toISOString().replace('T', ' ').slice(0, 19)}Z`;
}
