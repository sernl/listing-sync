// The focus decision, lifted out of the two components that act on it so that
// it can be tested at all: this project's test lane is `environment: 'node'`
// with no jsdom, so a rule left inline in markup is a rule nothing checks.

import { describe, expect, it } from 'vitest';
import {
	afterBannerDismissed,
	afterToastDismissed,
	captureSlots,
	focusRegion
} from './focus-return';
import type { FocusableRegion } from './focus-return';

describe('closing a toast', () => {
	it('returns focus to whatever the toast interrupted', () => {
		expect(afterToastDismissed({ previous: true, region: true })).toBe('previous');
	});

	it('falls back to the region when that control has since gone', () => {
		expect(afterToastDismissed({ previous: false, region: true })).toBe('region');
	});

	it('leaves focus alone when there is no region either', () => {
		expect(afterToastDismissed({ previous: false, region: false })).toBe('none');
	});

	it('prefers the interrupted control even when a region is also offered', () => {
		expect(afterToastDismissed({ previous: true, region: false })).toBe('previous');
	});
});

describe('closing a banner', () => {
	it('puts focus in the region the banner sat in', () => {
		expect(afterBannerDismissed({ previous: false, region: true })).toBe('region');
	});

	it('does not send focus back to where the seller was before the banner', () => {
		// The distinguishing case: the same offer that sends a toast to
		// `previous` must send a banner to `region`.
		expect(afterBannerDismissed({ previous: true, region: true })).toBe('region');
		expect(afterToastDismissed({ previous: true, region: true })).toBe('previous');
	});

	it('leaves focus alone when the region has gone too', () => {
		expect(afterBannerDismissed({ previous: true, region: false })).toBe('none');
	});
});

describe('which toasts have a control to return to recorded', () => {
	it('records the first toast when nothing is held yet', () => {
		expect(captureSlots([1], [])).toEqual({ add: [1], drop: [] });
	});

	it('records a second toast arriving while the first still stands', () => {
		// The defect this replaces: one slot for the whole stack was occupied by
		// the first toast, so the second was never asked and closing it returned
		// focus to whatever the first had interrupted.
		expect(captureSlots([1, 2], [1])).toEqual({ add: [2], drop: [] });
	});

	it('does not ask again for a toast already recorded', () => {
		// An implementation that re-captured on every store change would answer
		// with whichever control had taken focus since — on a full stack that is
		// the toast's own close button.
		expect(captureSlots([1, 2], [1, 2])).toEqual({ add: [], drop: [] });
	});

	it('releases a toast that has left the stack', () => {
		expect(captureSlots([2], [1, 2])).toEqual({ add: [], drop: [1] });
	});

	it('releases everything when the stack empties', () => {
		expect(captureSlots([], [1, 2])).toEqual({ add: [], drop: [1, 2] });
	});

	it('records and releases in one answer', () => {
		// A sweep that drops one toast as another is raised: an implementation
		// that returned early on either half would leave a stale element held or
		// a new toast unasked.
		expect(captureSlots([2, 3], [1, 2])).toEqual({ add: [3], drop: [1] });
	});
});

/** A region that records what was done to it, which is the whole reason
 *  `focusRegion` is typed over three members rather than over `HTMLElement`:
 *  this lane has no DOM. */
function stubRegion(attributes: Record<string, string> = {}): FocusableRegion & {
	readonly calls: string[];
	readonly attributes: Record<string, string>;
} {
	const calls: string[] = [];
	const held = { ...attributes };
	return {
		calls,
		attributes: held,
		hasAttribute: (name) => name in held,
		setAttribute: (name, value) => {
			held[name] = value;
			calls.push(`set ${name}=${value}`);
		},
		focus: () => {
			calls.push('focus');
		}
	};
}

describe('moving focus to a region', () => {
	it('makes a region with no tabindex focusable, then focuses it', () => {
		// Dropping the `setAttribute` leaves `focus()` a no-op on a plain
		// `<main>`, which is the silent half of this failure.
		const region = stubRegion();
		focusRegion(region);
		expect(region.calls).toEqual(['set tabindex=-1', 'focus']);
	});

	it('leaves a tabindex the page already set alone', () => {
		// An unconditional stamp would take a region the page deliberately made
		// tabbable back out of the tab order.
		const region = stubRegion({ tabindex: '0' });
		focusRegion(region);
		expect(region.attributes.tabindex).toBe('0');
	});

	it('focuses in that branch too', () => {
		// The guard is on the attribute, never on the move: an implementation
		// that focused only where it had set the attribute would do nothing at
		// all the second time a region is returned to.
		const region = stubRegion({ tabindex: '-1' });
		focusRegion(region);
		expect(region.calls).toEqual(['focus']);
	});
});
