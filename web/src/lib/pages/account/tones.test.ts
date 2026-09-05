import { describe, expect, it } from 'vitest';
import { badgeTone, paddleBadge } from './tones';

describe('the older pill vocabulary in the badge’s', () => {
	it('carries the three tinted values through unchanged', () => {
		expect(badgeTone('ok')).toBe('ok');
		expect(badgeTone('run')).toBe('run');
		expect(badgeTone('bad')).toBe('bad');
	});

	// The badge has no untinted tone, so the untinted modifier has to land
	// somewhere; grey is the tone that already means "nothing to report".
	it('sends the untinted one to grey rather than tinting it', () => {
		expect(badgeTone('mut')).toBe('soon');
	});
});

describe('a Paddle status in the badge’s vocabulary', () => {
	it('is green while money is arriving', () => {
		expect(paddleBadge('active')).toBe('ok');
		expect(paddleBadge('trialing')).toBe('ok');
	});

	it('marks the two states that mean it has stopped', () => {
		expect(paddleBadge('past_due')).toBe('run');
		expect(paddleBadge('canceled')).toBe('bad');
	});

	it('leaves a status neither module recognises untinted', () => {
		expect(paddleBadge('paused')).toBe('soon');
		expect(paddleBadge('something_paddle_added_later')).toBe('soon');
	});
});
