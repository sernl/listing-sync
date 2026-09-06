// The focus decision, lifted out of the two components that act on it so that
// it can be tested at all: this project's test lane is `environment: 'node'`
// with no jsdom, so a rule left inline in markup is a rule nothing checks.

import { describe, expect, it } from 'vitest';
import { afterBannerDismissed, afterToastDismissed } from './focus-return';

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
