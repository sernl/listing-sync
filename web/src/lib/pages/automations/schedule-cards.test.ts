// The one property the founder's decision turns on: nothing on the two
// schedule cards can be operated.
//
// Every other test in this slice is on a pure module, and none of them says
// anything about the pages. Deleting `disabled` from the four Sharing controls
// and the Save button left all 53 of them passing, which meant the decision
// itself — Sharing ships with its settings laid out and inert, and Sync's
// schedule card is inert because no route stores a cadence — was unguarded.
//
// This reads the page source rather than mounting a component, because the
// suite runs in `node` with no DOM and adding one would mean editing a shared
// config. The trade is deliberate: it catches the mutation that matters
// (a control quietly becoming live) at the cost of being blind to a control
// renamed or removed, which the id assertions below cover instead.

import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

function page(path: string): string {
	return readFileSync(new URL(`../../../routes/${path}`, import.meta.url), 'utf8');
}

const SHARING = page('automations/sharing/+page.svelte');
const SYNC = page('sync/+page.svelte');

/** The opening tag that starts at `anchor`, up to its first `>`. */
function tag(source: string, anchor: string): string {
	const start = source.indexOf(anchor);
	expect(start, `no control matching ${anchor}`).toBeGreaterThan(-1);
	const end = source.indexOf('>', start);
	expect(end, `unterminated tag at ${anchor}`).toBeGreaterThan(start);
	return source.slice(start, end);
}

/** A call that reaches the server with a change. Matched as `api.<verb>(`
 *  rather than as a bare word, because prose in a comment is not a call —
 *  "the resolved selection" is not `api.resolve(`. */
const WRITES = [
	'api.createSyncRequest(',
	'api.declareAuthorship(',
	'api.revoke(',
	'api.revokeDevice(',
	'api.resolve(',
	'api.noCounterpart(',
	'api.createJob('
];

describe('the Sharing card cannot be operated', () => {
	for (const control of ['<select id="share-when"', '<input id="share-time"', '<select id="share-labels"']) {
		it(`${control} is disabled`, () => {
			expect(tag(SHARING, control)).toContain('disabled');
		});
	}

	it('the include toggle is disabled', () => {
		expect(tag(SHARING, '<Toggle label="Include this marketplace"')).toContain('disabled');
	});

	it('the save button is disabled and states why', () => {
		const save = tag(SHARING, '<Button tier="primary"');
		expect(save).toContain('disabled');
		expect(save).toContain('reason=');
	});

	it('reaches the server for reads only, so nothing here can change anything', () => {
		for (const write of WRITES) {
			expect(SHARING).not.toContain(write);
		}
	});
});

describe('the Sync schedule card cannot be operated', () => {
	it('the cadence select is disabled', () => {
		expect(tag(SYNC, '<select id="sync-cadence"')).toContain('disabled');
	});

	it('both toggles are disabled', () => {
		expect(tag(SYNC, '<Toggle label="Include this marketplace"')).toContain('disabled');
		expect(tag(SYNC, '<Toggle\n\t\t\t\t\t\t\t\tlabel="Ask me before')).toContain('disabled');
	});

	it('the save button is disabled and states why', () => {
		const save = tag(SYNC, '<Button tier="primary"');
		expect(save).toContain('disabled');
		expect(save).toContain('reason=');
	});

	it('writes nothing: the run list and the question count are both reads', () => {
		for (const write of WRITES) {
			expect(SYNC).not.toContain(write);
		}
	});
});

describe('neither card claims a setting that is stored', () => {
	it('says on the card itself that nothing here is in force', () => {
		expect(SHARING).toContain('SETTINGS_ARE_A_PREVIEW');
		expect(SYNC).toContain('SETTINGS_ARE_A_PREVIEW');
	});

	it('leaves every schedule toggle off, rather than inventing a default per page', () => {
		expect(tag(SHARING, '<Toggle label="Include this marketplace"')).toContain('checked={false}');
		expect(tag(SYNC, '<Toggle label="Include this marketplace"')).toContain('checked={false}');
	});
});
