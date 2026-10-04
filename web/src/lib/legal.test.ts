import { describe, expect, it } from 'vitest';
import { TERMS_VERSION as GENERATED_TERMS_VERSION } from '$lib/generated/legal';
import {
	CONSENT_REFUSAL,
	TERMS_VERSION,
	consentBody,
	consentReady,
	socialConsentError
} from './legal';

describe('the two sign-up boxes', () => {
	it('is ready only once both are ticked', () => {
		expect(consentReady({ terms: false, age: false })).toBe(false);
		expect(consentReady({ terms: true, age: false })).toBe(false);
		expect(consentReady({ terms: false, age: true })).toBe(false);
		expect(consentReady({ terms: true, age: true })).toBe(true);
	});

	it('has no body to send until both are ticked', () => {
		expect(consentBody({ terms: false, age: false })).toBeNull();
		expect(consentBody({ terms: true, age: false })).toBeNull();
		expect(consentBody({ terms: false, age: true })).toBeNull();
		expect(consentBody({ terms: true, age: true })).not.toBeNull();
	});

	it('carries the generated terms version by default', () => {
		expect(TERMS_VERSION).toBe(GENERATED_TERMS_VERSION);
		expect(consentBody({ terms: true, age: true })?.version).toBe(TERMS_VERSION);
		expect(consentBody({ terms: true, age: true }, '2027-01-01')?.version).toBe('2027-01-01');
	});

	it('records box (a) as both the terms and ownership, and box (b) as age', () => {
		expect(consentBody({ terms: true, age: true })).toEqual({
			terms_privacy: true,
			ip_ownership: true,
			age_18: true,
			version: TERMS_VERSION
		});
	});
});

describe('social sign-in error codes', () => {
	it('names each agreement refusal and ignores the rest', () => {
		expect(socialConsentError('signup_disabled')).toMatch(/sign-up page/);
		expect(socialConsentError('consent_unavailable')).toMatch(/not created/);
		expect(socialConsentError('unable_to_create_user')).toBe(CONSENT_REFUSAL);
		expect(socialConsentError('something_else')).toBeNull();
		expect(socialConsentError(null)).toBeNull();
	});
});
