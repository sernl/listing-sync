import { describe, expect, it } from 'vitest';
import { ApiFailure } from '$lib/api';
import {
	NOT_SIGNED_BACK_IN,
	signBackInRefusal,
	signedOutHere,
	whereYouAre
} from './machine-here';

describe('where the seller is', () => {
	// The founder's ask in as many words: every platform must say which
	// machine he is on and whether he is in the app. One sentence, because
	// the same one is shown in the strip's account title, the phone's account
	// avatar and on the Preferences panel, and three wordings of one fact
	// would read as three facts.
	it('names the machine when the app knows which one it is', () => {
		expect(whereYouAre({ inApp: true, device: { id: 'dev_1', name: 'SM-N975F' } })).toBe(
			'You are on SM-N975F in the Teachouse app.'
		);
	});

	it('still says it is the app when it cannot name the machine', () => {
		// An application older than the two fields, and a check-in that has not
		// landed. Naming no machine is true; inventing one is not.
		const said = whereYouAre({ inApp: true, device: null });
		expect(said).toBe('You are in the Teachouse app on this machine.');
		expect(said).toContain('Teachouse app');
	});

	it('says a browser is a browser, and names no machine there', () => {
		const said = whereYouAre({ inApp: false, device: null });
		expect(said).toBe('You are in a browser, not the Teachouse app.');
		// A browser is no machine at all: the registry holds none for it, and a
		// name here would claim it holds marketplace logins that it cannot.
		expect(whereYouAre({ inApp: false, device: { id: 'dev_1', name: 'Laptop' } })).toBe(said);
	});
});

describe('the banner over a machine that was signed out', () => {
	it('dates the act and names the act that undoes it', () => {
		// 1 October 2025, in UTC.
		const said = signedOutHere(Date.UTC(2025, 9, 1, 7, 11));
		expect(said).toContain('on 1 October');
		expect(said).toContain('Sign it back in');
	});

	it('omits the date rather than guessing one', () => {
		// The registry read has not landed. A wrong date reads as a different
		// act, and the seller's own act is what the sentence is about.
		const said = signedOutHere(null);
		expect(said).toBe(
			'This machine was signed out of your account. Sign it back in to use marketplace logins here.'
		);
		expect(said).not.toMatch(/ on /);
	});
});

describe('what a refused sign-in says', () => {
	// The refusal that matters is the machine cap, which names a plan and a
	// number that we do not. "Try again" over it would be advice to repeat
	// something that refuses identically for ever.
	it("carries the server's own sentence wherever it sent one", () => {
		const capped = new ApiFailure(422, {
			status: 422,
			errors: [{ message: 'your plan allows 2 machines; sign one out first' }]
		});
		expect(signBackInRefusal(capped)).toBe('your plan allows 2 machines; sign one out first');
	});

	it('falls back to our own words where nothing readable came back', () => {
		expect(signBackInRefusal(new Error('fetch failed'))).toBe(NOT_SIGNED_BACK_IN);
		expect(signBackInRefusal(null)).toBe(NOT_SIGNED_BACK_IN);
		// A failing response with no structured body at all: `ApiFailure` writes
		// itself "request failed with 500", which is a sentence for us.
		expect(signBackInRefusal(new ApiFailure(500, null))).toBe(NOT_SIGNED_BACK_IN);
	});
});
