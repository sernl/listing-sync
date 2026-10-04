import { describe, expect, it } from 'vitest';
import { ApiFailure } from '$lib/http';
import { safeNext } from './intent';
import {
	FRESH_MS,
	GOODBYE_URL,
	REAUTH_RETURN,
	REAUTH_URL,
	confirmationAccepted,
	deletionBody,
	deletionFailure,
	needsFreshSignIn
} from './delete-account';

const NOW = Date.UTC(2026, 9, 4, 9);
const MINUTE = 60 * 1000;

function refusedWith(status: number, refusal: string, message = 'Server words.') {
	return new ApiFailure(status, {
		status,
		errors: [{ message, kind: 'validation', detail: { refusal } }]
	});
}

describe('needsFreshSignIn', () => {
	it('never asks a password account to sign in again: the password is the proof', () => {
		expect(needsFreshSignIn({ hasPassword: true, signedInAt: null }, NOW)).toBe(false);
		expect(
			needsFreshSignIn({ hasPassword: true, signedInAt: new Date(NOW - 60 * MINUTE) }, NOW)
		).toBe(false);
	});

	it('asks any other account for a sign-in younger than the window, less a minute to type', () => {
		const signedIn = (ago: number) =>
			needsFreshSignIn({ hasPassword: false, signedInAt: new Date(NOW - ago) }, NOW);
		expect(signedIn(MINUTE)).toBe(false);
		expect(signedIn(FRESH_MS - MINUTE - 1)).toBe(false);
		expect(signedIn(FRESH_MS - MINUTE)).toBe(true);
		expect(signedIn(FRESH_MS + MINUTE)).toBe(true);
		expect(needsFreshSignIn({ hasPassword: false, signedInAt: null }, NOW)).toBe(true);
	});
});

describe('confirmationAccepted', () => {
	it('takes the account name in any case, or DELETE in capitals', () => {
		expect(confirmationAccepted('maths-corner', 'maths-corner')).toBe(true);
		expect(confirmationAccepted(' Maths-Corner ', 'maths-corner')).toBe(true);
		expect(confirmationAccepted('DELETE', 'maths-corner')).toBe(true);
		expect(confirmationAccepted('DELETE', null)).toBe(true);
	});

	it('refuses anything else', () => {
		expect(confirmationAccepted('delete', 'maths-corner')).toBe(false);
		expect(confirmationAccepted('maths', 'maths-corner')).toBe(false);
		expect(confirmationAccepted('', null)).toBe(false);
		expect(confirmationAccepted('   ', 'maths-corner')).toBe(false);
	});
});

describe('deletionBody', () => {
	it('leaves out an empty password and a blank reason', () => {
		expect(deletionBody(' DELETE ', '', '   ')).toEqual({ confirm: 'DELETE' });
		expect(deletionBody('maths-corner', 'pw', ' Moving on ')).toEqual({
			confirm: 'maths-corner',
			password: 'pw',
			reason: 'Moving on'
		});
	});
});

describe('deletionFailure', () => {
	it('sends a stale sign-in to sign in again', () => {
		expect(deletionFailure(refusedWith(403, 'reauthenticate'))).toEqual({ kind: 'reauthenticate' });
	});

	it("shows the server's words, against the field they are about", () => {
		expect(deletionFailure(refusedWith(403, 'password', 'Wrong.'))).toEqual({
			kind: 'show',
			message: 'Wrong.',
			field: 'password'
		});
		expect(deletionFailure(refusedWith(422, 'password_required')).kind).toBe('show');
		expect(deletionFailure(refusedWith(422, 'confirmation'))).toMatchObject({ field: 'confirm' });
		expect(deletionFailure(refusedWith(422, 'operator'))).toMatchObject({ field: null });
	});

	it('says nothing was deleted when the failure is not an answer at all', () => {
		const shown = deletionFailure(new TypeError('Failed to fetch'));
		expect(shown).toMatchObject({ kind: 'show', field: null });
		expect(shown.kind === 'show' && shown.message).toContain('Nothing was deleted');
	});
});

describe('the round trips', () => {
	it('comes back to this page with the sheet open, through a next the sign-in page accepts', () => {
		const next = new URL(REAUTH_URL, 'https://dash.teachouse.io').searchParams.get('next');
		expect(next).toBe(REAUTH_RETURN);
		expect(safeNext(next)).toBe(REAUTH_RETURN);
	});

	it('leaves for the landing page’s goodbye', () => {
		expect(GOODBYE_URL).toBe('https://teachouse.io/?deleted=1');
	});
});
