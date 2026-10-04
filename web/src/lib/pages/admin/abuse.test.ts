import { describe, expect, it } from 'vitest';
import {
	ACTIONS,
	FLAG_KIND_LABEL,
	REASON_MAX,
	SIGNAL_KIND_LABEL,
	STANDING_TONE,
	actionBody,
	actionFlag,
	actionIsCurrent,
	banSentence,
	flagsQuery,
	patchFlags,
	patchRow,
	reasonProblem,
	type AbuseFlagView,
	type AbuseFlagsView,
	type AbuseOrgRow,
	type AbuseOrgView
} from './abuse';

const flag = (over: Partial<AbuseFlagView> = {}): AbuseFlagView => ({
	id: 'f1',
	kind: 'shared_shop',
	score: 40,
	reason: 'Same TPT store as Other School',
	created_at: 1_000,
	resolved_at: null,
	resolved_by: null,
	action: 'none',
	action_reason: null,
	mail_sent_at: null,
	mail_error: null,
	...over
});

const view = (over: Partial<AbuseOrgView> = {}): AbuseOrgView => ({
	org: 'o1',
	name: 'Kauri School',
	slug: 'kauri',
	created_at: 500,
	standing: 'none',
	flags: [flag()],
	signals: [],
	linked: [],
	...over
});

const row = (over: Partial<AbuseOrgRow> = {}): AbuseOrgRow => ({
	org: 'o1',
	name: 'Kauri School',
	slug: 'kauri',
	score: 40,
	kinds: ['shared_shop'],
	signal_kinds: ['shop_digest'],
	linked_orgs: 1,
	standing: 'none',
	open_flags: 1,
	last_flagged_at: 1_000,
	flag: 'f1',
	...over
});

describe('the words', () => {
	it('names every flag and signal kind in plain words', () => {
		expect(FLAG_KIND_LABEL.shared_device).toBe('Same device');
		expect(FLAG_KIND_LABEL.disposable_email).toBe('Throwaway email');
		expect(SIGNAL_KIND_LABEL.payment_fingerprint).toBe('Card');
		expect(SIGNAL_KIND_LABEL.user_agent_hash).toBe('Browser');
	});

	it('colours a standing by how serious it is', () => {
		expect(STANDING_TONE.none).toBe('soon');
		expect(STANDING_TONE.warn).toBe('warn');
		expect(STANDING_TONE.ban).toBe('bad');
	});

	it('says in one sentence what a ban does', () => {
		expect(banSentence('Kauri School')).toBe(
			'Banning signs everyone in Kauri School out, suspends the account and refuses its email, shops, devices and cards for 24 months.'
		);
	});
});

describe('flagsQuery', () => {
	it('always sends the state and only a search that says something', () => {
		expect(flagsQuery('open', '')).toBe('state=open');
		expect(flagsQuery('all', '   ')).toBe('state=all');
		expect(flagsQuery('open', ' a+b@example.com ')).toBe('state=open&q=a%2Bb%40example.com');
		expect(flagsQuery('all', 'tpt:123')).toBe('state=all&q=tpt%3A123');
	});
});

describe('reasonProblem', () => {
	it('asks a ban for a reason and lets the rest go without one', () => {
		expect(reasonProblem('ban', '')).not.toBeNull();
		expect(reasonProblem('ban', '   ')).not.toBeNull();
		expect(reasonProblem('ban', 'Four accounts on one store')).toBeNull();
		for (const action of ['dismiss', 'warn', 'limit'] as const) {
			expect(reasonProblem(action, '')).toBeNull();
		}
	});

	it('holds every reason to the server’s limit', () => {
		expect(reasonProblem('warn', 'x'.repeat(REASON_MAX))).toBeNull();
		expect(reasonProblem('warn', 'x'.repeat(REASON_MAX + 1))).not.toBeNull();
	});
});

describe('actionBody', () => {
	it('sends the reason trimmed, or null when there is none', () => {
		expect(actionBody('  shared card ')).toEqual({ reason: 'shared card' });
		expect(actionBody('   ')).toEqual({ reason: null });
	});
});

describe('actionFlag', () => {
	it('acts on the newest flag, and on nothing when there is none', () => {
		expect(actionFlag(view({ flags: [flag({ id: 'new' }), flag({ id: 'old' })] }))).toBe('new');
		expect(actionFlag(view({ flags: [] }))).toBeNull();
	});
});

describe('actionIsCurrent', () => {
	it('is true only when nothing is open and the standing is already there', () => {
		const resolved = flag({ resolved_at: 2_000, action: 'warn' });
		expect(actionIsCurrent('warn', view({ standing: 'warn', flags: [resolved] }))).toBe(true);
		expect(actionIsCurrent('dismiss', view({ standing: 'none', flags: [resolved] }))).toBe(true);
		expect(actionIsCurrent('ban', view({ standing: 'warn', flags: [resolved] }))).toBe(false);
		expect(actionIsCurrent('warn', view({ standing: 'warn' }))).toBe(false);
		expect(ACTIONS).toEqual(['dismiss', 'warn', 'limit', 'ban']);
	});
});

describe('patchRow', () => {
	it('redraws the row from what an action answered', () => {
		const answered = view({
			standing: 'ban',
			flags: [
				flag({ id: 'f2', resolved_at: 3_000, action: 'ban' }),
				flag({ resolved_at: 3_000, action: 'ban' })
			]
		});
		expect(patchRow(row(), answered)).toEqual(
			row({ standing: 'ban', open_flags: 0, score: 0, kinds: ['shared_shop'] })
		);
	});

	it('sums and lists what is still open, oldest kind first', () => {
		const answered = view({
			flags: [
				flag({ id: 'f3', kind: 'shared_device', score: 25, created_at: 3_000 }),
				flag({ id: 'f2', kind: 'shared_shop', score: 10, created_at: 2_000 }),
				flag({ id: 'f1', kind: 'shared_shop', score: 40, created_at: 1_000 }),
				flag({ id: 'f0', kind: 'signup_burst', resolved_at: 900, created_at: 800 })
			]
		});
		expect(patchRow(row(), answered)).toMatchObject({
			open_flags: 3,
			score: 75,
			kinds: ['shared_shop', 'shared_device']
		});
	});

	it('leaves other organisations alone', () => {
		const other = row({ org: 'o2' });
		expect(patchRow(other, view({ standing: 'ban' }))).toBe(other);
	});

	it('patches the matching row of a whole list', () => {
		const list: AbuseFlagsView = {
			counters: {
				open_orgs: 2,
				open_flags: 2,
				warned: 0,
				limited: 0,
				banned: 0,
				banned_identities: 0,
				signals: 4
			},
			query: 'none',
			orgs: [row(), row({ org: 'o2' })]
		};
		const patched = patchFlags(list, view({ standing: 'limit' }));
		expect(patched.orgs.map((entry) => entry.standing)).toEqual(['limit', 'none']);
		expect(patched.counters).toBe(list.counters);
	});
});
