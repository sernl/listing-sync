// What gets recorded when a page fails to draw.
//
// A function rather than a line inside the boundary's handler, because the
// sentence a reader will be sent when they report a blank screen is the whole
// value of the record, and a line reachable only by making a page throw is a
// line nothing cheap can check.

/**
 * One line naming the route that failed and what it threw.
 *
 * The route is first because it is the part that is always known: an error
 * thrown during rendering may be anything at all, including a string, a plain
 * object, or nothing, and a record that assumed `Error` would print
 * "undefined" for the case a reader most needs to describe.
 */
export function renderFailureReport(error: unknown, route: string): string {
	return `the page at ${route} could not be drawn: ${causeOf(error)}`;
}

/** What was thrown, in as many words as it actually carries. */
function causeOf(error: unknown): string {
	if (error instanceof Error && error.message.trim().length > 0) {
		return error.message;
	}
	if (typeof error === 'string' && error.trim().length > 0) {
		return error;
	}
	// Deliberately not `String(error)`: that renders null as "null" and a plain
	// object as "[object Object]", both of which read as a cause and are not
	// one. Saying nothing was carried is the honest answer.
	return 'it threw something carrying no message';
}
