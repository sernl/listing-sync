import { describe, expect, it } from 'vitest';
import { gateLabel } from '$lib/gates';

describe('gateLabel', () => {
	it('labels every gate the server can send', () => {
		expect(gateLabel('awaiting_seller_signin')).toBe('you signing in again, so this can be checked');
		expect(gateLabel('ReauthRequired')).toBe('you signing in again');
	});

	it('reads as a sentence after the prefix the render site adds', () => {
		expect(`blocked on ${gateLabel('election')}`).toBe('blocked on your answer');
		expect(`blocked on ${gateLabel('cover_missing')}`).toBe('blocked on a missing cover image');
	});

	// The label completes someone else's sentence, so it is a noun phrase
	// rather than a sentence of its own: "blocked on Waiting on the
	// marketplace" is what a sentence here would read as.
	it('names what is awaited rather than restating that something is', () => {
		expect(`blocked on ${gateLabel('awaiting_marketplace_answer')}`).toBe(
			'blocked on an answer from the marketplace'
		);
	});

	// The map is exhaustive over the generated union, so this is unreachable
	// through the API. It exists because a client running against a newer
	// server would otherwise render nothing where a gate should be, and a
	// blank pill is worse than an unfamiliar word.
	it('falls back to the raw gate rather than rendering nothing', () => {
		expect(gateLabel('a_gate_from_a_newer_server')).toBe('a_gate_from_a_newer_server');
	});
});
