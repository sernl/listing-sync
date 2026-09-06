import { describe, expect, it } from 'vitest';
import type { DeviceView } from '$lib/api';
import { type BrowserSession, matchNote, merge, platformOf, sessionLabel } from './device-merge';

const WINDOWS =
	'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0 Safari/537.36';
const MAC =
	'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15';
const ANDROID =
	'Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0 Mobile Safari/537.36';
const IPHONE =
	'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1';

function device(id: string, name: string, os: string): DeviceView {
	return {
		id,
		name,
		os,
		arch: 'x86_64',
		app_version: '0.1.0',
		first_seen_at: 1_756_000_000_000,
		last_seen_at: 1_756_000_000_000,
		revoked_at: null,
		wipe_outstanding: false,
		sessions: []
	};
}

function session(token: string, userAgent: string | null, ip = '203.0.113.7'): BrowserSession {
	return { id: `id-${token}`, token, userAgent, ipAddress: ip };
}

describe('platformOf', () => {
	it('reads the platform the device registry stores, in the same vocabulary', () => {
		expect(platformOf(WINDOWS)).toBe('windows');
		expect(platformOf(MAC)).toBe('macos');
		expect(platformOf('Mozilla/5.0 (X11; Linux x86_64)')).toBe('linux');
	});

	it('does not read an Android agent as Linux, nor an iPhone as macOS', () => {
		// Both strings genuinely contain the more general token, which is why
		// the order of the tests inside platformOf is what it is.
		expect(ANDROID).toContain('Linux');
		expect(platformOf(ANDROID)).toBe('android');
		expect(IPHONE).toContain('Mac OS X');
		expect(platformOf(IPHONE)).toBe('ios');
	});

	it('answers null rather than guessing when there is nothing to read', () => {
		expect(platformOf(null)).toBeNull();
		expect(platformOf(undefined)).toBeNull();
		expect(platformOf('curl/8.9.1')).toBeNull();
	});
});

describe('merge', () => {
	it('attaches a sign-in to a device when exactly one of each claims the platform', () => {
		const current = session('tok-win', WINDOWS);
		const { rows, orphans } = merge([device('d1', 'founder-pc', 'windows')], [current], 'tok-win');
		expect(rows).toHaveLength(1);
		expect(rows[0].confidence).toBe('matched');
		expect(rows[0].session?.token).toBe('tok-win');
		expect(rows[0].isCurrent).toBe(true);
		expect(orphans).toHaveLength(0);
	});

	it('refuses to guess when two machines run the same operating system', () => {
		const { rows, orphans } = merge(
			[device('d1', 'founder-pc', 'windows'), device('d2', 'studio-pc', 'windows')],
			[session('tok-win', WINDOWS)],
			null
		);
		expect(rows.map((row) => row.confidence)).toEqual(['ambiguous', 'ambiguous']);
		expect(rows.every((row) => row.session === null)).toBe(true);
		expect(
			orphans.map((orphan) => orphan.session.token),
			'an unattributed sign-in still gets a row of its own to be signed out from'
		).toEqual(['tok-win']);
	});

	it('refuses to guess when one machine has two browsers signed in', () => {
		const { rows, orphans } = merge(
			[device('d1', 'founder-pc', 'windows')],
			[session('tok-a', WINDOWS), session('tok-b', WINDOWS)],
			'tok-b'
		);
		expect(rows[0].confidence).toBe('ambiguous');
		expect(orphans.map((orphan) => orphan.session.token)).toEqual(['tok-a', 'tok-b']);
		expect(
			orphans.find((orphan) => orphan.session.token === 'tok-b')?.isCurrent,
			'the seller must still be able to see which sign-in is the one they are using'
		).toBe(true);
	});

	it('says a device has no browser sign-in rather than attaching an unrelated one', () => {
		const { rows, orphans } = merge(
			[device('d1', 'founder-pc', 'windows')],
			[session('tok-mac', MAC)],
			null
		);
		expect(rows[0].confidence).toBe('unmatched');
		expect(rows[0].session).toBeNull();
		expect(orphans).toHaveLength(1);
	});

	it('matches each platform independently', () => {
		const { rows, orphans } = merge(
			[device('d1', 'founder-pc', 'windows'), device('d2', 'studio-mac', 'macos')],
			[session('tok-win', WINDOWS), session('tok-mac', MAC), session('tok-phone', ANDROID)],
			'tok-mac'
		);
		expect(rows.map((row) => [row.device.id, row.confidence, row.session?.token])).toEqual([
			['d1', 'matched', 'tok-win'],
			['d2', 'matched', 'tok-mac']
		]);
		expect(rows[1].isCurrent).toBe(true);
		expect(
			orphans.map((orphan) => orphan.session.token),
			'the phone has no registered device, so it is listed on its own'
		).toEqual(['tok-phone']);
	});

	it('lists a registered phone as a machine rather than as a stray browser sign-in', () => {
		// The founder's complaint, as a test. The Android app signs in by
		// navigating its own window to the console, so its session is a browser
		// session with an Android user agent and nothing else. Until the app
		// registered a device there was no row to claim it, and Settings could
		// only show it under "Browser sign-ins".
		const { rows, orphans } = merge(
			[device('d1', 'Google Pixel 8', 'android')],
			[session('tok-phone', ANDROID)],
			null
		);
		expect(rows.map((row) => [row.device.name, row.confidence, row.session?.token])).toEqual([
			['Google Pixel 8', 'matched', 'tok-phone']
		]);
		expect(
			orphans,
			'a phone that appears in both lists at once tells the seller they have two things'
		).toHaveLength(0);
	});

	it('leaves an unreadable user agent unattributed rather than assigning it', () => {
		const { rows, orphans } = merge(
			[device('d1', 'founder-pc', 'windows')],
			[session('tok-unknown', 'curl/8.9.1')],
			null
		);
		expect(rows[0].confidence).toBe('unmatched');
		expect(orphans).toHaveLength(1);
	});

	it('handles a device whose reported operating system is not one we know', () => {
		const { rows } = merge([device('d1', 'a-box', 'freebsd')], [session('tok', WINDOWS)], null);
		expect(rows[0].confidence).toBe('unmatched');
	});
});

describe('the words beside a row', () => {
	it('never claims certainty the join does not have', () => {
		expect(matchNote('matched')).toMatch(/Matched/);
		expect(matchNote('ambiguous')).toMatch(/cannot say which/);
		expect(matchNote('unmatched')).toMatch(/No browser sign-in/);
	});

	it('names a sign-in from the only two columns better-auth stores', () => {
		expect(sessionLabel(session('t', WINDOWS, '203.0.113.7'))).toBe(
			'Browser on windows from 203.0.113.7'
		);
		expect(sessionLabel({ id: 'i', token: 't' })).toBe('Unrecognised browser');
	});
});
