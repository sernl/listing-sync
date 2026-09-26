import { describe, expect, it } from 'vitest';
import { examplePrice } from './seller-rules';

const usd = (minor_units: number) => ({ Paid: { minor_units, currency: 'Usd' } });
const gbp = (minor_units: number) => ({ Paid: { minor_units, currency: 'Gbp' } });

// The worked example on the Pricing page has to say what the server's
// `convert_price` would say, or the arrow teaches the seller the wrong rule.
describe('examplePrice', () => {
	it('takes a half penny up at nearest', () => {
		// USD 5.01 × 0.5 = 2.505 → 2.51
		expect(examplePrice(usd(501), '0.5', 'Nearest', 'Tpt', 'Tes')).toEqual(gbp(251));
	});

	it('charms against the unrounded value, never below it', () => {
		// USD 240.08 × 0.05 = 12.004: nearest gives 12.00, the charm 12.99.
		expect(examplePrice(usd(24008), '0.05', 'Nearest', 'Tpt', 'Tes')).toEqual(gbp(1200));
		expect(examplePrice(usd(24008), '0.05', 'UpToCharm', 'Tpt', 'Tes')).toEqual(gbp(1299));
		// An exact whole amount stays in its own pound.
		expect(examplePrice(usd(1000), '1', 'UpToCharm', 'Tpt', 'Tes')).toEqual(gbp(1099));
	});

	it('keeps a free resource free', () => {
		expect(examplePrice('Free', '0.75', 'UpToCharm', 'Tpt', 'Tes')).toBe('Free');
	});

	it('shows nothing where the server would refuse', () => {
		expect(examplePrice(gbp(500), '0.75', 'Nearest', 'Tpt', 'Tes')).toBeNull();
		expect(examplePrice(usd(500), 'abc', 'Nearest', 'Tpt', 'Tes')).toBeNull();
		expect(examplePrice(usd(1), '0.1', 'Nearest', 'Tpt', 'Tes')).toBeNull();
		expect(examplePrice(usd(500), '0.75', 'Nearest', 'Tpt', 'Etsy')).toBeNull();
	});
});
