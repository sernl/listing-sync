import { describe, expect, it } from 'vitest';
import { CAPTCHA_HEADER, captchaOptions, captchaPending, readSiteKey } from './captcha';

describe('the configured site key', () => {
	it('is absent unless a non-blank string was given', () => {
		expect(readSiteKey(undefined)).toBeNull();
		expect(readSiteKey('')).toBeNull();
		expect(readSiteKey('   ')).toBeNull();
		expect(readSiteKey(1)).toBeNull();
	});

	it('is the trimmed key when one was', () => {
		expect(readSiteKey('  0x4AAAAAAA  ')).toBe('0x4AAAAAAA');
	});
});

describe('the fetch options a guarded call carries', () => {
	it('are absent with no solved challenge, so an ungated build sends what it always sent', () => {
		expect(captchaOptions(null)).toBeUndefined();
	});

	it('carry the solved challenge under the header the plugin reads', () => {
		expect(captchaOptions('solved')).toEqual({ headers: { [CAPTCHA_HEADER]: 'solved' } });
		expect(CAPTCHA_HEADER).toBe('x-captcha-response');
	});
});

describe('the submit gate', () => {
	it('never holds a build that configured no site key', () => {
		expect(captchaPending(null, null)).toBe(false);
	});

	it('holds until a configured challenge is solved, and releases once it is', () => {
		expect(captchaPending('0x4AAAAAAA', null)).toBe(true);
		expect(captchaPending('0x4AAAAAAA', 'solved')).toBe(false);
	});
});
