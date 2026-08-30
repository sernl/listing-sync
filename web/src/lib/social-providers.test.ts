import { describe, expect, it } from 'vitest';
import { SOCIAL_PROVIDERS, readEnabledProviders } from './social-providers';

describe('the providers a build renders', () => {
	it('are none when the list is absent, blank, or not a string', () => {
		expect(readEnabledProviders(undefined)).toEqual([]);
		expect(readEnabledProviders('')).toEqual([]);
		expect(readEnabledProviders('   ')).toEqual([]);
		expect(readEnabledProviders(',, ,')).toEqual([]);
		expect(readEnabledProviders(1)).toEqual([]);
	});

	it('are exactly the named ones', () => {
		expect(readEnabledProviders('google').map((p) => p.id)).toEqual(['google']);
		expect(readEnabledProviders('microsoft').map((p) => p.id)).toEqual(['microsoft']);
		expect(readEnabledProviders('google,microsoft').map((p) => p.id)).toEqual([
			'google',
			'microsoft'
		]);
	});

	it('tolerate the spacing and casing a hand-written variable arrives with', () => {
		expect(readEnabledProviders(' Google , MICROSOFT ').map((p) => p.id)).toEqual([
			'google',
			'microsoft'
		]);
	});

	it('drop a name no provider answers to, rather than offering a button that cannot work', () => {
		expect(readEnabledProviders('google,github,apple').map((p) => p.id)).toEqual(['google']);
		expect(readEnabledProviders('github')).toEqual([]);
	});

	it('are listed once each, in the catalogue order, however the list is written', () => {
		expect(readEnabledProviders('microsoft,google,google').map((p) => p.id)).toEqual([
			'google',
			'microsoft'
		]);
	});

	it('carry the label the button shows', () => {
		expect(readEnabledProviders('google,microsoft')).toEqual([
			{ id: 'google', label: 'Google' },
			{ id: 'microsoft', label: 'Microsoft' }
		]);
	});

	it('never exceed the catalogue', () => {
		expect(readEnabledProviders('google,microsoft')).toHaveLength(SOCIAL_PROVIDERS.length);
	});
});
