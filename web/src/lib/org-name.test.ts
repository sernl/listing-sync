import { describe, expect, it } from 'vitest';
import { NAME_MAX_CHARS, checkOrgName } from './org-name';

describe('the organisation-name check', () => {
	it('accepts the trimmed name, which is the one the server stores', () => {
		expect(checkOrgName('  Riverbend Resources \n')).toEqual({
			accepted: true,
			name: 'Riverbend Resources'
		});
	});

	it('refuses a name that is only whitespace as empty', () => {
		for (const raw of ['', '   ', '\t\n']) {
			const verdict = checkOrgName(raw);
			expect(verdict.accepted, `refused: ${JSON.stringify(raw)}`).toBe(false);
			expect(verdict.accepted === false && verdict.problem).toBe('empty');
		}
	});

	it('counts the bound after trimming', () => {
		const longest = 'e'.repeat(NAME_MAX_CHARS);
		expect(checkOrgName(` ${longest} `).accepted).toBe(true);
	});

	it('refuses one character past the bound, and says what the bound is', () => {
		const verdict = checkOrgName('e'.repeat(NAME_MAX_CHARS + 1));
		expect(verdict.accepted).toBe(false);
		expect(verdict.accepted === false && verdict.problem).toBe('too-long');
		expect(verdict.accepted === false && verdict.message).toContain(String(NAME_MAX_CHARS));
	});

	// The divergence a UTF-16 count would introduce: each of these is one
	// character to the server and two code units to `String.length`, so a
	// length-counting client would refuse at half the real bound.
	it('counts characters rather than UTF-16 code units', () => {
		const astral = '\u{1f4da}'.repeat(NAME_MAX_CHARS);
		expect(astral.length).toBe(NAME_MAX_CHARS * 2);
		expect(checkOrgName(astral).accepted).toBe(true);
		expect(checkOrgName('\u{1f4da}'.repeat(NAME_MAX_CHARS + 1)).accepted).toBe(false);
	});
});
