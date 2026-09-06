// The dismissal rule, and the one thing about the control that draws it which
// no other lane holds.
//
// The rule is pure, so it is tested directly rather than through a rendered
// component: there is no component-testing library here and `menu-dismissal`
// establishes the precedent. What a test cannot reach is the Done control's
// tier, which is a prop in the markup and the whole of the founder's second
// complaint, so that is read out of the source in the manner of
// `dismissal.test.ts`.

import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import {
	closes,
	doneLabel,
	pressedInside,
	returnsFocus,
	type PickerEvent
} from './picker-dismissal';

function picker(): string {
	return readFileSync(new URL('FacetPicker.svelte', import.meta.url), 'utf8');
}

describe('what closes an open picker', () => {
	it('closes on Escape', () => {
		expect(closes({ kind: 'escape' })).toBe(true);
	});

	it('closes on a press outside its own box', () => {
		expect(closes({ kind: 'press', inside: false })).toBe(true);
	});

	it('stays open on a press inside its own box', () => {
		// A picker that closed here would shut the moment the seller reached
		// for a tick, which is every press they make while it is open.
		expect(closes({ kind: 'press', inside: true })).toBe(false);
	});

	it('closes on a tap on the scrim', () => {
		expect(closes({ kind: 'scrim' })).toBe(true);
	});

	it('closes on Done', () => {
		expect(closes({ kind: 'done' })).toBe(true);
	});

	it('stays open when an option is ticked', () => {
		// The one answer that differs from `menu-dismissal`, where a choice
		// closes. Copying that rule across would shut this picker on the
		// seller's first tick, and they are choosing up to six.
		expect(closes({ kind: 'pick' })).toBe(false);
	});
});

describe('the reasons a picker can be given', () => {
	// Crude on purpose, in the manner of `dismissal.test.ts`: the type checker
	// makes `closes` answer every member of the union, and nothing makes this
	// file answer for one. A reason added upstream would otherwise arrive with
	// no test saying what it does.
	const ANSWERED: PickerEvent['kind'][] = ['escape', 'press', 'scrim', 'done', 'pick'];

	it('are all answered above', () => {
		const source = readFileSync(new URL('picker-dismissal.ts', import.meta.url), 'utf8');
		const declared = [...source.matchAll(/kind:\s*'([a-z]+)'/g)].map((match) => match[1]);
		expect([...new Set(declared)].sort()).toEqual([...ANSWERED].sort());
	});

	it('every one of them gives an answer', () => {
		const given = ANSWERED.map((kind) =>
			closes(kind === 'press' ? { kind, inside: false } : { kind })
		);
		expect(given.every((answer) => typeof answer === 'boolean')).toBe(true);
	});
});

describe("the Done control's words", () => {
	it('counts nothing before anything is chosen', () => {
		expect(doneLabel(0)).toBe('Done');
	});

	it('counts one without pluralising it', () => {
		expect(doneLabel(1)).toBe('Done, 1 chosen');
	});

	it('counts what is chosen', () => {
		expect(doneLabel(3)).toBe('Done, 3 chosen');
	});
});

describe('where a press landed', () => {
	// The scrim is drawn inside the picker's own element and a press on it is
	// inside: a rule that closed on that press would remove the sheet under
	// the finger, and the tap's click would land on whatever the dimming
	// covered. The scrim closes on its own click instead, read for below.
	const option = {} as EventTarget;
	const scrim = {} as EventTarget;
	const elsewhere = {} as EventTarget;
	const box = { contains: (node: Node | null) => node === option || node === scrim };

	it("is inside when the press is on one of the picker's own options", () => {
		expect(pressedInside(option, box)).toBe(true);
	});

	it('is inside when the press is on the scrim the box contains', () => {
		expect(pressedInside(scrim, box)).toBe(true);
	});

	it('is outside when the press is on the page', () => {
		expect(pressedInside(elsewhere, box)).toBe(false);
	});

	it('is outside when the picker has no element to be inside of', () => {
		expect(pressedInside(option, null)).toBe(false);
	});

	it('is outside when the press has no target', () => {
		expect(pressedInside(null, box)).toBe(false);
	});
});

describe('where focus goes when the picker closes', () => {
	it('returns to the search box when it was inside the picker', () => {
		expect(returnsFocus(true, false)).toBe(true);
	});

	it('stays where a press outside put it', () => {
		expect(returnsFocus(false, false)).toBe(false);
	});

	it('is not returned on a coarse pointer, which would raise the keyboard', () => {
		expect(returnsFocus(true, true)).toBe(false);
	});

	it("is asked with the pointer's coarseness in hand", () => {
		// The rule can be right while the component answers Done with a bare
		// `search.focus()`, which is a keyboard raised for a box the seller
		// has just finished with.
		expect(picker()).toContain("matchMedia('(pointer: coarse)')");
		expect(picker()).toContain('returnsFocus(held, coarse)');
	});
});

describe('how the picker answers a press', () => {
	// The rule above can be right while nothing asks it, and nothing asking it
	// is exactly what the founder met: only the button dismissed the picker. A
	// press outside this component's own tree is visible at the window and
	// nowhere else, so that is what is read for.
	it('listens for a press at the window', () => {
		expect(/<svelte:window[^>]*onpointerdown/.test(picker())).toBe(true);
	});

	it('asks whether the press landed inside its own box', () => {
		expect(picker()).toContain("closes({ kind: 'press', inside })");
		expect(picker()).toContain('pressedInside(event.target, anchor)');
	});

	it('closes on the scrim by its click and not by the press on it', () => {
		// `onpointerdown` here is the reading that looks consistent with the
		// window, and it is the one that lets the tap fall through to the
		// control under the dimming.
		expect(/class="fp-scrim"[^>]*onclick=/.test(picker())).toBe(true);
		expect(/class="fp-scrim"[^>]*onpointerdown/.test(picker())).toBe(false);
		expect(picker()).toContain("closes({ kind: 'scrim' })");
	});
});

describe('how the picker answers a key', () => {
	it('ticks an option on Enter rather than submitting the form', () => {
		// The browser's own answer to Enter on a checkbox inside a form is the
		// form's implicit submission, which here is Create or Save.
		const source = picker();
		const at = source.indexOf("event.key === 'Enter'");
		expect(at).toBeGreaterThan(-1);
		expect(source.slice(at, at + 160)).toContain(
			'event.preventDefault();\n\t\t\tevent.target.click();'
		);
	});
});

describe('where the sheet leaves the search box', () => {
	// The phone sheet rises to 72vh from the bottom, and the filter is typed
	// into a box that can sit anywhere on the page under it. The box is
	// brought to the top on opening, below the width the sheet is drawn at and
	// nowhere else, so the guard and the stylesheet must name one breakpoint.
	it('brings the box to the top of the screen on opening', () => {
		expect(picker()).toContain("scrollIntoView({ block: 'start' })");
	});

	it('does so below the width the sheet is drawn at', () => {
		const source = picker();
		const guard = source.match(/matchMedia\('\(max-width: (\d+)px\)'\)/);
		const sheet = source.match(/@media \(max-width: (\d+)px\)/);
		expect(guard?.[1]).toBeDefined();
		expect(guard?.[1]).toBe(sheet?.[1]);
	});
});

describe('the control the picker draws it with', () => {
	// The founder's complaint was a grey pill the same colour as everything
	// around it -- `small` on `Button` is `--control-h-sm` at 30px over
	// `var(--card)`, and `tier="primary"` is `.cta`'s filled jade at the full
	// control height. Nothing else in this repository would catch a restyle
	// putting the pill back.
	function doneControl(): string {
		const source = picker();
		const at = source.indexOf('<Button');
		const end = source.indexOf('</Button>', at);
		if (at === -1 || end === -1) {
			throw new Error('FacetPicker.svelte draws no Button');
		}
		return source.slice(at, end);
	}

	it('is a primary button', () => {
		expect(doneControl()).toContain('tier="primary"');
	});

	it('is not the small one', () => {
		expect(doneControl()).not.toContain('small');
	});

	it('says how many are chosen', () => {
		expect(doneControl()).toContain('doneLabel(chosen.length)');
	});
});
