import { describe, expect, it } from 'vitest';
import {
	ORG_CUSTOM_DATA_KEY,
	PADDLE_SCRIPT,
	readPaddleConfig,
	subscriptionTone
} from './paddle';

describe('the checkout a build offers', () => {
	it('is none unless both the token and the price were given', () => {
		expect(readPaddleConfig(undefined, undefined, undefined)).toBeNull();
		expect(readPaddleConfig('live_abc', undefined, undefined)).toBeNull();
		expect(readPaddleConfig(undefined, 'pri_abc', undefined)).toBeNull();
		expect(readPaddleConfig('', 'pri_abc', undefined)).toBeNull();
		expect(readPaddleConfig('live_abc', '   ', undefined)).toBeNull();
		expect(readPaddleConfig(1, 2, undefined)).toBeNull();
	});

	it('trims what it was given', () => {
		expect(readPaddleConfig('  live_abc  ', '  pri_abc  ', undefined)).toEqual({
			clientToken: 'live_abc',
			priceId: 'pri_abc',
			environment: 'production'
		});
	});

	it("takes Paddle's own default when no environment was named", () => {
		expect(readPaddleConfig('live_abc', 'pri_abc', '')?.environment).toBe('production');
		expect(readPaddleConfig('live_abc', 'pri_abc', '  ')?.environment).toBe('production');
	});

	it('reads a named environment however it was written', () => {
		expect(readPaddleConfig('t', 'p', 'sandbox')?.environment).toBe('sandbox');
		expect(readPaddleConfig('t', 'p', ' SANDBOX ')?.environment).toBe('sandbox');
		expect(readPaddleConfig('t', 'p', 'Production')?.environment).toBe('production');
	});

	it('offers nothing at all for an environment it does not recognise, rather than charging a real card', () => {
		expect(readPaddleConfig('t', 'p', 'sandbx')).toBeNull();
		expect(readPaddleConfig('t', 'p', 'staging')).toBeNull();
	});
});

describe('what the checkout tells the server', () => {
	it('uses the key the webhook reads a tenant from', () => {
		expect(ORG_CUSTOM_DATA_KEY).toBe('org');
	});

	it("loads Paddle from Paddle's own domain and nowhere else", () => {
		expect(PADDLE_SCRIPT).toBe('https://cdn.paddle.com/paddle/v2/paddle.js');
	});
});

describe('a subscription status', () => {
	it('reads as healthy while it is being paid for', () => {
		expect(subscriptionTone('active')).toBe('ok');
		expect(subscriptionTone('trialing')).toBe('ok');
	});

	it('separates a late payment from a cancellation', () => {
		expect(subscriptionTone('past_due')).toBe('run');
		expect(subscriptionTone('canceled')).toBe('bad');
	});

	it('is untinted for a word this client does not know, rather than guessed', () => {
		expect(subscriptionTone('paused')).toBe('mut');
		expect(subscriptionTone('something_paddle_added')).toBe('mut');
		expect(subscriptionTone('')).toBe('mut');
	});
});
