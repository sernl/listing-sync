import { describe, expect, it } from 'vitest';
import { badgeTone } from './tones';

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
