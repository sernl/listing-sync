import { describe, expect, it } from 'vitest';
import type { MarketplaceSyncSettingView } from '$lib/api';
import {
	activityEntries,
	cadenceOptions,
	heldCadence,
	lastPullLine,
	openQuestionsLabel,
	runRows,
	syncCards,
	templateChoices
} from './sync';
import type { TemplateHead } from '$lib/pages/templates/api';
import { connection, job } from './fixtures.test-support';

const HOUR = 3_600_000;
const SIX_HOURS = 21_600;
const DAILY = 86_400;
const WEEKLY = 604_800;

function setting(
	over: Partial<MarketplaceSyncSettingView> = {}
): MarketplaceSyncSettingView {
	return {
		inventory: 'Tpt',
		enabled: true,
		interval_secs: DAILY,
		minimum_secs: SIX_HOURS,
		last_pull_at: null,
		publish_to: [],
		template_id: null,
		...over
	};
}

describe('a run row', () => {
	it('leads with where the run was sent rather than with its identifier', () => {
		const [row] = runRows([job({ inventory: 'Tes' })], 0);
		expect(row.inventory).toBe('Tes');
	});

	it('keeps the identifier and the age on the meta line', () => {
		const [row] = runRows([job({ job: 'abcdef1234567890', created_at: 0 })], HOUR);
		expect(row.meta).toContain('abcdef12');
		expect(row.meta).toContain('1 h ago');
	});

	it('opens the run', () => {
		const [row] = runRows([job({ job: 'j-7' })], 0);
		expect(row.href).toBe('/sync/j-7');
	});

	it('hands the raw instant on, so the template renders it in the seller’s own locale', () => {
		const [row] = runRows([job({ created_at: 1234 })], 0);
		expect(row.at).toBe(1234);
	});
});

describe('the cadences a plan reaches', () => {
	it('offers the three §5 names and nothing else', () => {
		expect(cadenceOptions(SIX_HOURS).map((option) => option.label)).toEqual([
			'Every 6 hours',
			'Daily',
			'Weekly'
		]);
	});

	// Disabled rather than absent: a seller asking why they cannot check
	// hourly is owed the figure, and an option that is not there answers
	// nothing.
	it('disables what is under the floor and names the floor', () => {
		const [six, daily, weekly] = cadenceOptions(DAILY);
		expect(six.enabled).toBe(false);
		expect(six.reason).toContain('daily');
		expect(daily.enabled).toBe(true);
		expect(daily.reason).toBeNull();
		expect(weekly.enabled).toBe(true);
	});

	// An unread entitlement and a marketplace with no stored row both arrive
	// as null. Refusing every cadence there would disable a paying seller's
	// controls for as long as the read took; the plan that pulls at no cadence
	// at all is the page's own gate, said once.
	it('leaves every cadence standing where no floor is known', () => {
		expect(cadenceOptions(null).every((option) => option.enabled)).toBe(true);
		expect(cadenceOptions(null).every((option) => option.reason === null)).toBe(true);
	});

	it('keeps the stored cadence where the plan still reaches it', () => {
		expect(heldCadence(WEEKLY, SIX_HOURS)).toBe(WEEKLY);
	});

	// What a downgrade leaves behind. A select whose value matches only a
	// disabled option shows a setting that cannot be saved, so the card lands
	// on the fastest cadence the plan does reach.
	it('lifts a stored cadence the plan has since moved above', () => {
		expect(heldCadence(SIX_HOURS, DAILY)).toBe(DAILY);
	});

	it('opens a card with no stored cadence on the fastest the plan reaches', () => {
		expect(heldCadence(0, DAILY)).toBe(DAILY);
		expect(heldCadence(0, null)).toBe(SIX_HOURS);
	});
});

describe('the pull cards', () => {
	// The route answers one row per marketplace a device can read, padded for
	// the ones with nothing stored, so which marketplaces can be pulled from
	// is the server's answer. Etsy is reached by an official API and is
	// absent from it; a card here would offer a cadence nothing keeps.
	it('draws a card for every served row and none for a marketplace with no row', () => {
		const cards = syncCards(
			[connection('Tpt')],
			[setting({ inventory: 'Tes' }), setting({ inventory: 'Tpt' })]
		);
		expect(cards.map((card) => card.inventory)).toEqual(['Tpt', 'Tes']);
	});

	it('says which of the served marketplaces the seller actually has', () => {
		const cards = syncCards(
			[connection('Tpt')],
			[setting({ inventory: 'Tpt' }), setting({ inventory: 'Tes' })]
		);
		expect(cards.find((card) => card.inventory === 'Tpt')?.connected).toBe(true);
		expect(cards.find((card) => card.inventory === 'Tes')?.connected).toBe(false);
	});

	it('reads a disconnected marketplace as not connected rather than as present', () => {
		const cards = syncCards([connection('Tpt', { state: 'unlinked' })], [setting()]);
		expect(cards[0].connected).toBe(false);
	});

	it('hands each card its own served row', () => {
		const cards = syncCards([connection('Tpt')], [setting({ interval_secs: WEEKLY })]);
		expect(cards[0].setting.interval_secs).toBe(WEEKLY);
	});

	it('states never pulled rather than leaving a blank', () => {
		expect(lastPullLine(null, 0)).toBe('Never pulled');
		expect(lastPullLine(0, HOUR)).toBe('1 h ago');
	});
});

describe('the activity log', () => {
	it('renders the server’s own sentence rather than composing one', () => {
		const [entry] = activityEntries(
			[{ at: 0, line: '“Fractions Pack” pulled from TPT', href: '/resources/p-1' }],
			HOUR
		);
		expect(entry.what).toBe('“Fractions Pack” pulled from TPT');
		expect(entry.at).toBe('1 h ago');
		expect(entry.href).toBe('/resources/p-1');
	});

	// Two lines can share an instant and neither carries an identifier, so the
	// key has to be more than the clock.
	it('keys two lines of the same instant apart', () => {
		const entries = activityEntries(
			[
				{ at: 0, line: 'one', href: null },
				{ at: 0, line: 'two', href: null }
			],
			0
		);
		expect(entries[0].id).not.toBe(entries[1].id);
		expect(entries[0].href).toBeUndefined();
	});
});

describe('the open-questions control', () => {
	it('carries the figure where there is one', () => {
		expect(openQuestionsLabel(3)).toBe('Open questions (3)');
		expect(openQuestionsLabel(0)).toBe('Open questions (0)');
	});

	it('drops the figure rather than claiming zero where it could not be read', () => {
		expect(openQuestionsLabel(null)).toBe('Open questions');
	});
});

describe('the templates a pull rule may fill from', () => {
	const head = (over: Partial<TemplateHead> = {}): TemplateHead => ({
		id: 'tpl-1',
		name: 'Worksheets',
		description: null,
		scope: null,
		created_at: 0,
		updated_at: 0,
		...over
	});

	it('offers a generic template under any rule, named as it was saved', () => {
		expect(templateChoices([head()], [])).toEqual([
			{ id: 'tpl-1', label: 'Worksheets', reason: null }
		]);
	});

	it('names the marketplace a scoped template was written for', () => {
		const [choice] = templateChoices([head({ scope: 'Tpt' })], ['Tpt']);
		expect(choice.label).toBe('Worksheets — TPT');
		expect(choice.reason).toBeNull();
	});

	it('offers a template the rule does not publish to, with the reason, rather than hiding it', () => {
		// The server answers 422 for this pair, so the option is disabled and
		// says why: a seller hunting a template they know they saved is worse
		// served by a shorter list than by a reason.
		const [choice] = templateChoices([head({ scope: 'Tpt' })], ['Tes']);
		expect(choice.reason).toMatch(/does not publish to/);
	});

	it('re-decides every option when the ticked targets change', () => {
		const heads = [head({ scope: 'Tpt' }), head({ id: 'tpl-2', scope: 'Tes' })];
		expect(templateChoices(heads, ['Tpt']).map((choice) => choice.reason === null)).toEqual([
			true,
			false
		]);
		expect(templateChoices(heads, ['Tpt', 'Tes']).every((choice) => choice.reason === null)).toBe(
			true
		);
	});
});
