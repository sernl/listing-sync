// What a page knows about a read it depends on.
//
// Four states rather than the two a bare array carries, because "the read
// failed" and "the read came back empty" are different facts about the
// seller's account and only the last two may render content. Telling a seller
// they have connected no marketplace when the request simply failed is the
// regression this module exists to make unrepresentable: the panel asks for a
// state, and there is no way to ask for one that conflates the two.
//
// Pure, so it tests without a component.

export type ReadState<Row> =
	| { kind: 'pending' }
	| { kind: 'failed' }
	| { kind: 'empty' }
	| { kind: 'rows'; rows: readonly Row[] };

/** The state of one read, from the two flags a page already keeps.
 *
 * `failed` is answered before `loaded`, because a read that failed is
 * finished: a page that checked `loaded` first would show a spinner for ever
 * on the one path where there is something to say. */
export function readState<Row>(
	loaded: boolean,
	failed: boolean,
	rows: readonly Row[]
): ReadState<Row> {
	if (failed) {
		return { kind: 'failed' };
	}
	if (!loaded) {
		return { kind: 'pending' };
	}
	return rows.length === 0 ? { kind: 'empty' } : { kind: 'rows', rows };
}

export interface PanelCopy {
	title: string;
	body: string;
}

/** What the settings panel says when it has no marketplace to show, or null
 *  where it does and the panel renders the marketplace instead.
 *
 * The failed wording states what did and did not happen, because the seller's
 * next question after "could not be read" is whether anything was changed. */
export function panelCopy<Row>(state: ReadState<Row>, what: string): PanelCopy | null {
	switch (state.kind) {
		case 'pending':
			return { title: 'Reading your marketplaces…', body: '' };
		case 'failed':
			return {
				title: 'Your marketplaces could not be read',
				body:
					'This page cannot say which marketplaces you have connected, so it is not showing ' +
					'settings for any of them. Nothing has been changed, and your connections are ' +
					'unaffected.'
			};
		case 'empty':
			return {
				title: 'No marketplace connected',
				body: `${what} acts on a marketplace, so there is nothing to set up until one is connected.`
			};
		case 'rows':
			return null;
	}
}

/** What the marketplace column says in place of its rows. Null where it has
 *  rows to draw. */
export function columnCopy<Row>(state: ReadState<Row>): string | null {
	switch (state.kind) {
		case 'pending':
			return 'Reading your marketplaces…';
		case 'failed':
			return 'Your marketplaces could not be read.';
		case 'empty':
			return 'Connect a marketplace and it appears here.';
		case 'rows':
			return null;
	}
}
