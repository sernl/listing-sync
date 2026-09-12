// The Scheduling page's model: what a schedule reads as in the seller's own
// words, what a half-filled form is still missing, and the two instants the
// list prints. Pure, so it tests without a component.
//
// The arithmetic a schedule actually turns on — when the next tick falls, and
// what a clock change does to it — is the scheduler's and arrives already
// worked out on `next_run_at`. Nothing here recomputes it: a second
// implementation in the browser would disagree with the server twice a year,
// and the seller would believe whichever one they were looking at.

import type {
	PublishIntent,
	ScheduleBody,
	ScheduleRepeat,
	ScheduleSelection,
	ScheduleView
} from '$lib/api';
import { agoLabel } from '$lib/elapsed';
import type { InventoryId } from '$lib/generated/vocab';

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/** What the page is for, said once at the top. */
export const WHAT_SCHEDULING_IS =
	'Publish a set of resources to the marketplaces you choose, at a time you choose, ' +
	'without being at your computer.';

/** Where the work happens, which is the part a seller has to know: the tick
 *  is ours, the marketplace request is their own machine's. */
export const RUNS_ON_YOUR_COMPUTER =
	'We queue the work at the time you set. Your own computer sends it at its next check-in, ' +
	'signed in as you, which is the only place your marketplace login is kept.';

export const NO_DEVICE_TITLE = 'A schedule needs a computer to run on';

export const NO_DEVICE_BODY =
	'Install the Teachouse app to give the schedule a computer to run on. Your marketplace ' +
	'sign-in stays there.';

/** Why the republish rule cannot do everything it says on one marketplace.
 *
 * Stated on the control rather than discovered in a run report: `lower()`
 * refuses a live-to-live transition on Tes because the edit-published capture
 * is not taken, so the tick records the resource as skipped with this reason.
 * A seller who reads it first is not surprised by it afterwards. */
export const TES_CANNOT_REVISE =
	'Tes cannot yet change a listing that is already live, so a resource that changes is ' +
	'skipped there and the run says so. Every other marketplace is republished.';

export const NO_SCHEDULE_YET =
	'Nothing is scheduled. A schedule sends the same selection on a timetable, so a weekly ' +
	'drop takes one setup rather than one evening a week.';

/** How often a schedule comes round, in the order the form offers them. */
export const REPEATS: readonly { value: ScheduleRepeat; label: string }[] = [
	{ value: 'once', label: 'Once' },
	{ value: 'daily', label: 'Daily' },
	{ value: 'weekly', label: 'Weekly' }
];

/** Where a scheduled send leaves the listing, as the two cards read.
 *
 * Each carries a sentence rather than a word alone, because the choice is
 * between two consequences and a radio's label is a line. */
export const INTENTS: readonly { value: PublishIntent; word: string; line: string }[] = [
	{ value: 'draft', word: 'Draft', line: 'Created on the marketplace for you to check.' },
	{ value: 'live', word: 'Live', line: 'Created and published in one go.' }
];

/** The days, indexed as `Date.getDay` counts them, which is the numbering the
 *  wire's `weekday` carries. */
export const WEEKDAYS: readonly string[] = [
	'Sunday',
	'Monday',
	'Tuesday',
	'Wednesday',
	'Thursday',
	'Friday',
	'Saturday'
];

/** A minute of the day as a 24-hour clock, which is both what the seller
 *  reads and what a `<input type="time">` takes. */
export function clockText(minute: number): string {
	const held = Math.max(0, Math.min(24 * 60 - 1, Math.trunc(minute)));
	const hours = Math.floor(held / 60);
	const minutes = held % 60;
	return `${String(hours).padStart(2, '0')}:${String(minutes).padStart(2, '0')}`;
}

/** The minute a clock reading names, or null where it names none.
 *
 * Null rather than a fallback of midnight: a time field a browser left empty
 * is a form that is not finished, and saving it as 00:00 would schedule a
 * send for the middle of the night the seller never asked for. */
export function minuteOf(clock: string): number | null {
	const match = /^(\d{1,2}):(\d{2})$/.exec(clock.trim());
	if (match === null) {
		return null;
	}
	const hours = Number(match[1]);
	const minutes = Number(match[2]);
	if (hours > 23 || minutes > 59) {
		return null;
	}
	return hours * 60 + minutes;
}

/** When a schedule comes round, as a sentence: "Daily at 09:00
 *  Pacific/Auckland".
 *
 * The zone is named rather than assumed, because a seller who set the time on
 * a laptop in one country and reads this page in another is owed the clock the
 * schedule actually keeps. */
export function whenSentence(schedule: {
	repeat: ScheduleRepeat;
	weekday: number | null;
	at_minute_of_day: number;
	timezone: string;
}): string {
	const at = `at ${clockText(schedule.at_minute_of_day)} ${schedule.timezone}`;
	switch (schedule.repeat) {
		case 'once':
			return `Once, ${at}`;
		case 'daily':
			return `Daily ${at}`;
		case 'weekly': {
			const day = WEEKDAYS[schedule.weekday ?? -1];
			// A weekly schedule with no day is not a day this module picks: the
			// server stores one, and inventing Sunday here would print a
			// sentence the scheduler does not keep.
			return day === undefined ? `Weekly ${at}` : `Weekly on ${day} ${at}`;
		}
	}
}

/** How far off the next tick is, in the console's own age vocabulary read
 *  forwards. A schedule with nothing ahead of it says so rather than printing
 *  a dash. */
export function nextRunLine(at: number | null, now: number): string {
	if (at === null) {
		return 'Not scheduled';
	}
	const ahead = at - now;
	if (ahead <= 0) {
		return 'Due now';
	}
	if (ahead < MINUTE) {
		return 'in under a minute';
	}
	if (ahead < HOUR) {
		return `in ${Math.floor(ahead / MINUTE)} min`;
	}
	if (ahead < DAY) {
		return `in ${Math.floor(ahead / HOUR)} h`;
	}
	const days = Math.floor(ahead / DAY);
	return `in ${days} ${days === 1 ? 'day' : 'days'}`;
}

/** When it last came round. A schedule that has never run says so: "never" is
 *  a fact about the schedule, and a blank would read as a failed read. */
export function lastRunLine(at: number | null, now: number): string {
	return at === null ? 'Never run' : agoLabel(at, now);
}

/** What a schedule sends, in one phrase. */
export function selectionLine(selection: ScheduleSelection): string {
	if ('label' in selection) {
		return `Everything labelled ${selection.label}`;
	}
	const count = selection.products.length;
	return count === 1 ? '1 chosen resource' : `${count} chosen resources`;
}

/** The seller's own timezone, which the form opens on.
 *
 * `UTC` where the browser will not name one: the zone is stored on the server
 * and read back by the scheduler, so it has to be a zone rather than a
 * sentence about this computer. */
export function timezoneName(): string {
	const zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
	return typeof zone === 'string' && zone.length > 0 ? zone : 'UTC';
}

/** What the timezone line reads in full. */
export function timezoneLine(zone: string): string {
	return `Times are ${zone}.`;
}

/** Enough zones to set a schedule where the browser will not list them all.
 *  `Intl.supportedValuesOf` is the real answer and is what nearly every
 *  browser gives; this is the floor beneath it. */
const FALLBACK_ZONES: readonly string[] = [
	'UTC',
	'Europe/London',
	'Europe/Dublin',
	'America/New_York',
	'America/Chicago',
	'America/Denver',
	'America/Los_Angeles',
	'Australia/Sydney',
	'Pacific/Auckland'
];

/** Every zone the select offers, with the seller's own always among them. */
export function timezones(): string[] {
	// Guarded rather than called outright: the declaration is in the lib this
	// project compiles against, and an older webview that does not ship the
	// function would otherwise throw inside a select's option list.
	const held =
		typeof Intl.supportedValuesOf === 'function' ? Intl.supportedValuesOf('timeZone') : [];
	const zones = held.length > 0 ? held : [...FALLBACK_ZONES];
	const own = timezoneName();
	return zones.includes(own) ? zones : [own, ...zones];
}

/** The form's own state, which is not the wire shape: a half-filled form has
 *  a time that does not parse and a selection that names neither side, and
 *  `ScheduleBody` can represent neither. */
export interface ScheduleDraft {
	name: string;
	/** The label the selection names, or null where the seller is ticking
	 *  resources instead. */
	label: string | null;
	products: string[];
	inventories: InventoryId[];
	intent: PublishIntent;
	/** As the time input holds it, so an empty field stays empty. */
	clock: string;
	timezone: string;
	repeat: ScheduleRepeat;
	weekday: number;
	republishOnUpdate: boolean;
	enabled: boolean;
}

/** A new schedule's opening position: nine in the morning, this computer's
 *  zone, daily, live, switched on. */
export function blankDraft(zone: string): ScheduleDraft {
	return {
		name: '',
		label: null,
		products: [],
		inventories: [],
		intent: 'live',
		clock: '09:00',
		timezone: zone,
		repeat: 'daily',
		weekday: 1,
		republishOnUpdate: false,
		enabled: true
	};
}

/** One stored schedule, back in the form's own shape, so Edit opens on what
 *  the seller saved rather than on a blank. */
export function draftOf(schedule: ScheduleView): ScheduleDraft {
	const label = 'label' in schedule.selection ? schedule.selection.label : null;
	return {
		name: schedule.name,
		label,
		products: 'products' in schedule.selection ? [...schedule.selection.products] : [],
		inventories: [...schedule.inventories],
		intent: schedule.intent,
		clock: clockText(schedule.at_minute_of_day),
		timezone: schedule.timezone,
		repeat: schedule.repeat,
		weekday: schedule.weekday ?? 1,
		republishOnUpdate: schedule.republish_on_update,
		enabled: schedule.enabled
	};
}

/** Why the form cannot be saved yet, or null where it can.
 *
 * One sentence naming the one thing to do next, in the order the form is
 * filled: a seller told about the marketplaces before the name would have to
 * scroll back up past the field they had not filled. */
export function draftRefusal(draft: ScheduleDraft): string | null {
	if (draft.name.trim().length === 0) {
		return 'Give the schedule a name, so you can tell it from the others.';
	}
	if (draft.label === null && draft.products.length === 0) {
		return 'Choose a label, or tick the resources to send.';
	}
	if (draft.label !== null && draft.label.trim().length === 0) {
		return 'Choose a label, or tick the resources to send.';
	}
	if (draft.inventories.length === 0) {
		return 'Choose at least one marketplace to send to.';
	}
	if (minuteOf(draft.clock) === null) {
		return 'Set a time of day for the schedule to run at.';
	}
	return null;
}

/** The draft as the wire takes it. Only ever called on a draft `draftRefusal`
 *  has passed, so the time parses; the fallback keeps the signature total
 *  rather than admitting a throw into a form handler. */
export function scheduleBody(draft: ScheduleDraft): ScheduleBody {
	return {
		name: draft.name.trim(),
		selection:
			draft.label === null
				? { products: [...draft.products] }
				: { label: draft.label.trim() },
		inventories: [...draft.inventories],
		intent: draft.intent,
		at_minute_of_day: minuteOf(draft.clock) ?? 0,
		timezone: draft.timezone,
		repeat: draft.repeat,
		// Null on every repeat but weekly, so a schedule changed from weekly to
		// daily does not carry the day it used to keep.
		weekday: draft.repeat === 'weekly' ? draft.weekday : null,
		republish_on_update: draft.republishOnUpdate,
		enabled: draft.enabled
	};
}

/** What the delete confirm asks, naming the schedule so a misread row is
 *  caught before the press rather than after it. */
export function deletePrompt(name: string): string {
	return `Delete the schedule “${name}”? Listings it has already sent are not touched.`;
}
