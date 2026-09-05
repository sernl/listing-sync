import { describe, expect, it } from 'vitest';
import { columnCopy, panelCopy, readState } from './read-state';

describe('the state of a read', () => {
	it('is pending before it has finished', () => {
		expect(readState(false, false, [])).toEqual({ kind: 'pending' });
	});

	it('is failed rather than pending once it has failed', () => {
		expect(readState(false, true, [])).toEqual({ kind: 'failed' });
	});

	it('is failed rather than empty, which is the whole point of the type', () => {
		expect(readState(true, true, [])).toEqual({ kind: 'failed' });
	});

	it('tells an empty answer from rows', () => {
		expect(readState(true, false, [])).toEqual({ kind: 'empty' });
		expect(readState(true, false, ['a'])).toEqual({ kind: 'rows', rows: ['a'] });
	});
});

describe('what the settings panel says', () => {
	it('never says the seller connected nothing when the read failed', () => {
		const failed = panelCopy(readState(true, true, []), 'Sync');
		expect(failed?.title).toBe('Your marketplaces could not be read');
		expect(failed?.title).not.toContain('No marketplace');
		expect(failed?.body).toContain('Nothing has been changed');
	});

	it('says the seller connected nothing only when the read said so', () => {
		expect(panelCopy(readState(true, false, []), 'Sync')?.title).toBe('No marketplace connected');
	});

	it('names the automation in the empty wording, so two pages do not read alike', () => {
		expect(panelCopy(readState(true, false, []), 'Sharing')?.body).toContain('Sharing acts on');
		expect(panelCopy(readState(true, false, []), 'Sync')?.body).toContain('Sync acts on');
	});

	it('hands the panel back to the page once there are rows to draw', () => {
		expect(panelCopy(readState(true, false, ['a']), 'Sync')).toBeNull();
	});

	it('gives every state its own words, so none can be mistaken for another', () => {
		const titles = [
			panelCopy(readState(false, false, []), 'Sync')?.title,
			panelCopy(readState(true, true, []), 'Sync')?.title,
			panelCopy(readState(true, false, []), 'Sync')?.title
		];
		expect(new Set(titles).size).toBe(3);
	});
});

describe('what the marketplace column says', () => {
	it('distinguishes a failed read from an empty one', () => {
		expect(columnCopy(readState(true, true, []))).toBe('Your marketplaces could not be read.');
		expect(columnCopy(readState(true, false, []))).toBe(
			'Connect a marketplace and it appears here.'
		);
	});

	it('says nothing once it has rows', () => {
		expect(columnCopy(readState(true, false, ['a']))).toBeNull();
	});
});
