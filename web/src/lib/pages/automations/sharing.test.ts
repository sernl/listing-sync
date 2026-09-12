import { describe, expect, it } from 'vitest';
import type { ScheduleView } from '$lib/api';
import {
	blankDraft,
	clockText,
	draftOf,
	draftRefusal,
	lastRunLine,
	minuteOf,
	nextRunLine,
	scheduleBody,
	selectionLine,
	timezoneLine,
	timezoneName,
	whenSentence
} from './sharing';

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

function schedule(over: Partial<ScheduleView> = {}): ScheduleView {
	return {
		id: 's-1',
		name: 'Friday drop',
		selection: { label: 'Maths' },
		inventories: ['Tpt'],
		intent: 'live',
		at_minute_of_day: 540,
		timezone: 'Pacific/Auckland',
		repeat: 'daily',
		weekday: null,
		republish_on_update: false,
		enabled: true,
		next_run_at: null,
		last_run_at: null,
		...over
	};
}

describe('when a schedule comes round, in the seller’s words', () => {
	it('names the clock and the zone on a daily schedule', () => {
		expect(whenSentence(schedule())).toBe('Daily at 09:00 Pacific/Auckland');
	});

	it('names the day on a weekly one', () => {
		expect(whenSentence(schedule({ repeat: 'weekly', weekday: 5 }))).toBe(
			'Weekly on Friday at 09:00 Pacific/Auckland'
		);
	});

	it('says once rather than implying a repeat', () => {
		expect(whenSentence(schedule({ repeat: 'once' }))).toBe(
			'Once, at 09:00 Pacific/Auckland'
		);
	});

	// A weekly row the server sent with no day is a row this module may not
	// invent one for: printing "on Sunday" would state a timetable the
	// scheduler does not keep.
	it('leaves the day out of a weekly schedule that names none', () => {
		expect(whenSentence(schedule({ repeat: 'weekly', weekday: null }))).toBe(
			'Weekly at 09:00 Pacific/Auckland'
		);
	});
});

describe('the clock', () => {
	it('pads both halves, so 9:05 is not read as five past ninety', () => {
		expect(clockText(545)).toBe('09:05');
		expect(clockText(0)).toBe('00:00');
		expect(clockText(23 * 60 + 59)).toBe('23:59');
	});

	it('reads a time field back to the minute it names', () => {
		expect(minuteOf('09:00')).toBe(540);
		expect(minuteOf('23:59')).toBe(1439);
	});

	// An empty or impossible field is a form that is not finished. Answering
	// midnight would schedule a send the seller never asked for.
	it('answers nothing for a field that names no time', () => {
		expect(minuteOf('')).toBeNull();
		expect(minuteOf('25:00')).toBeNull();
		expect(minuteOf('09:70')).toBeNull();
	});
});

describe('the two instants a row prints', () => {
	it('counts forward to the next run', () => {
		expect(nextRunLine(3 * HOUR, 0)).toBe('in 3 h');
		expect(nextRunLine(2 * DAY, 0)).toBe('in 2 days');
		expect(nextRunLine(5 * MINUTE, 0)).toBe('in 5 min');
	});

	it('says due now rather than counting backwards past zero', () => {
		expect(nextRunLine(0, HOUR)).toBe('Due now');
	});

	it('says a schedule with nothing ahead of it is not scheduled', () => {
		expect(nextRunLine(null, 0)).toBe('Not scheduled');
	});

	it('states never rather than leaving a blank that reads as a failed read', () => {
		expect(lastRunLine(null, 0)).toBe('Never run');
		expect(lastRunLine(0, HOUR)).toBe('1 h ago');
	});
});

describe('what a schedule sends', () => {
	it('names the label', () => {
		expect(selectionLine({ label: 'Maths' })).toBe('Everything labelled Maths');
	});

	it('agrees the count with its noun', () => {
		expect(selectionLine({ products: ['a'] })).toBe('1 chosen resource');
		expect(selectionLine({ products: ['a', 'b'] })).toBe('2 chosen resources');
	});
});

describe('the form', () => {
	it('asks for the one thing missing, in the order the form is filled', () => {
		const draft = blankDraft('UTC');
		expect(draftRefusal(draft)).toContain('name');
		expect(draftRefusal({ ...draft, name: 'Friday drop' })).toContain('label');
		expect(
			draftRefusal({ ...draft, name: 'Friday drop', label: 'Maths' })
		).toContain('marketplace');
		expect(
			draftRefusal({
				...draft,
				name: 'Friday drop',
				label: 'Maths',
				inventories: ['Tpt'],
				clock: ''
			})
		).toContain('time');
	});

	it('passes a schedule that names all four', () => {
		expect(
			draftRefusal({
				...blankDraft('UTC'),
				name: 'Friday drop',
				label: 'Maths',
				inventories: ['Tpt']
			})
		).toBeNull();
	});

	it('carries the day only on a weekly schedule', () => {
		const draft = { ...draftOf(schedule()), weekday: 5 };
		expect(scheduleBody({ ...draft, repeat: 'weekly' }).weekday).toBe(5);
		expect(scheduleBody({ ...draft, repeat: 'daily' }).weekday).toBeNull();
	});

	it('sends the selection the seller is actually on', () => {
		const draft = draftOf(schedule({ selection: { products: ['p-1', 'p-2'] } }));
		expect(scheduleBody(draft).selection).toEqual({ products: ['p-1', 'p-2'] });
		expect(scheduleBody({ ...draft, label: 'Maths' }).selection).toEqual({ label: 'Maths' });
	});

	it('opens an edit on what was saved rather than on a blank', () => {
		const draft = draftOf(schedule({ repeat: 'weekly', weekday: 3, intent: 'draft' }));
		expect(draft.clock).toBe('09:00');
		expect(draft.weekday).toBe(3);
		expect(draft.intent).toBe('draft');
		expect(draft.timezone).toBe('Pacific/Auckland');
	});
});

describe('the timezone the schedule means', () => {
	it('names a zone rather than leaving the clock ambiguous', () => {
		expect(timezoneName().length).toBeGreaterThan(0);
	});

	it('writes the zone into a sentence', () => {
		expect(timezoneLine('Pacific/Auckland')).toBe('Times are Pacific/Auckland.');
	});
});
