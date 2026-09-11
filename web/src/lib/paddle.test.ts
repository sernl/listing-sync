import { describe, expect, it } from 'vitest';
import {
	ORG_CUSTOM_DATA_KEY,
	PADDLE_SCRIPT,
	planPriceKey,
	readPaddleConfig,
	readPriceMap,
	rungPriceKey,
	subscriptionTone
} from './paddle';

const MAP = '{"subscriber_monthly":"pri_m","subscriber_yearly":"pri_y","rung_20":"pri_20"}';

describe('the price map a build was given', () => {
	it('reads every key it carries', () => {
		expect(readPriceMap(MAP)).toEqual({
			subscriber_monthly: 'pri_m',
			subscriber_yearly: 'pri_y',
			rung_20: 'pri_20'
		});
	});

	it('trims the identifiers, which arrive from a deployment file', () => {
		expect(readPriceMap('{"rung_20":"  pri_20  "}')).toEqual({ rung_20: 'pri_20' });
	});

	it('is none for anything that is not an object of non-empty strings', () => {
		expect(readPriceMap(undefined)).toBeNull();
		expect(readPriceMap('')).toBeNull();
		expect(readPriceMap('not json')).toBeNull();
		expect(readPriceMap('["pri_a"]')).toBeNull();
		expect(readPriceMap('"pri_a"')).toBeNull();
		expect(readPriceMap('null')).toBeNull();
	});

	// Half a map renders some buttons and silently withholds others, which
	// reads as a broken page rather than as a deployment nobody configured.
	it('is none where any one entry is unusable, rather than dropping that entry', () => {
		expect(readPriceMap('{"rung_20":"pri_20","rung_50":""}')).toBeNull();
		expect(readPriceMap('{"rung_20":"pri_20","rung_50":null}')).toBeNull();
		expect(readPriceMap('{"rung_20":"pri_20","rung_50":5}')).toBeNull();
	});

	it('is none for an empty object, because there is nothing to sell', () => {
		expect(readPriceMap('{}')).toBeNull();
	});
});

describe('the keys a price is looked up under', () => {
	// The server's `--paddle-price-map` is keyed the same way, so a rename on
	// one side has to be a rename on both.
	it('names a plan and its cadence', () => {
		expect(planPriceKey('subscriber', 'monthly')).toBe('subscriber_monthly');
		expect(planPriceKey('subscriber', 'annual')).toBe('subscriber_yearly');
	});

	it('names a ladder rung by the volume it covers', () => {
		expect(rungPriceKey(20)).toBe('rung_20');
		expect(rungPriceKey(500)).toBe('rung_500');
	});
});

describe('the checkout a build offers', () => {
	it('is none unless both the token and the map were given', () => {
		expect(readPaddleConfig(undefined, undefined, undefined)).toBeNull();
		expect(readPaddleConfig('live_abc', undefined, undefined)).toBeNull();
		expect(readPaddleConfig(undefined, MAP, undefined)).toBeNull();
		expect(readPaddleConfig('', MAP, undefined)).toBeNull();
		expect(readPaddleConfig('live_abc', '   ', undefined)).toBeNull();
		expect(readPaddleConfig(1, 2, undefined)).toBeNull();
	});

	it('trims what it was given', () => {
		expect(readPaddleConfig('  live_abc  ', '{"rung_20":"pri_20"}', undefined)).toEqual({
			clientToken: 'live_abc',
			prices: { rung_20: 'pri_20' },
			environment: 'production'
		});
	});

	it("takes Paddle's own default when no environment was named", () => {
		expect(readPaddleConfig('live_abc', MAP, '')?.environment).toBe('production');
		expect(readPaddleConfig('live_abc', MAP, '  ')?.environment).toBe('production');
	});

	it('reads a named environment however it was written', () => {
		expect(readPaddleConfig('t', MAP, 'sandbox')?.environment).toBe('sandbox');
		expect(readPaddleConfig('t', MAP, ' SANDBOX ')?.environment).toBe('sandbox');
		expect(readPaddleConfig('t', MAP, 'Production')?.environment).toBe('production');
	});

	it('offers nothing at all for an environment it does not recognise, rather than charging a real card', () => {
		expect(readPaddleConfig('t', MAP, 'sandbx')).toBeNull();
		expect(readPaddleConfig('t', MAP, 'staging')).toBeNull();
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
