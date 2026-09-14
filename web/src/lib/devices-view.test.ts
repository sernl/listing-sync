import { describe, expect, it } from 'vitest';
import {
	bandNotice,
	CHECK_IN_CADENCE_MS,
	deviceFootnote,
	deviceRows,
	deviceStanding,
	deviceSummary,
	marketplaceRows,
	needingAttention,
	needingDeviceSignIn,
	QUIET_AFTER_MS,
	schedulesRunning,
	signInStates,
	type MarketplaceRow,
	type MarketplaceSignIn
} from './devices-view';
import { present } from './connection-status';
import type { ConnectionView, DeviceSessionView, DeviceView } from './api';
import type { Marketplace } from './generated/vocab';

const NOW = 1_000_000_000;

function session(
	marketplace: Marketplace,
	status: DeviceSessionView['status'] = 'connected',
	label: string | null = 'Miss Cooper'
): DeviceSessionView {
	return {
		marketplace,
		account_label: label,
		linked_at: NOW - 10_000,
		last_used_at: NOW - 5_000,
		status
	};
}

function device(partial: Partial<DeviceView> = {}): DeviceView {
	return {
		id: 'd1',
		name: 'staffroom-laptop',
		os: 'windows',
		arch: 'x86_64',
		app_version: '0.9.0',
		runs_sourced_payloads: true,
		first_seen_at: NOW - 100_000,
		last_seen_at: NOW - 1_000,
		revoked_at: null,
		wipe_outstanding: false,
		sessions: [],
		...partial
	};
}

function connection(
	marketplace: Marketplace,
	status: ConnectionView['status'] = 'connected'
): ConnectionView {
	return {
		id: `c-${marketplace}`,
		marketplace,
		state: 'linked',
		status,
		created_at: 1,
		updated_at: 1
	};
}

/** Looked up rather than destructured by position: the band's order is a
 *  ranked map and a re-rank must not silently retarget a test. */
function of(states: readonly MarketplaceSignIn[], marketplace: Marketplace): MarketplaceSignIn {
	const found = states.find((entry) => entry.marketplace === marketplace);
	if (found === undefined) {
		throw new Error(`no state for ${marketplace}`);
	}
	return found;
}

describe('a device standing', () => {
	it('tolerates one missed check-in and reports the second', () => {
		expect(deviceStanding(device({ last_seen_at: NOW - CHECK_IN_CADENCE_MS }), NOW)).toBe(
			'checking_in'
		);
		expect(deviceStanding(device({ last_seen_at: NOW - QUIET_AFTER_MS }), NOW)).toBe(
			'checking_in'
		);
		expect(deviceStanding(device({ last_seen_at: NOW - QUIET_AFTER_MS - 1 }), NOW)).toBe('quiet');
	});

	it('reads a revoked machine as signed out however recently it spoke', () => {
		expect(
			deviceStanding(device({ last_seen_at: NOW, revoked_at: NOW - 10 }), NOW)
		).toBe('signed_out');
	});
});

describe('the summary', () => {
	it('counts what is current, what is quiet and what is still to be wiped', () => {
		const rows = deviceRows(
			[
				device({ id: 'a', sessions: [session('Tpt')] }),
				device({ id: 'b', last_seen_at: NOW - QUIET_AFTER_MS - 1 }),
				device({ id: 'c', revoked_at: NOW - 5, wipe_outstanding: true, sessions: [session('Tes')] })
			],
			NOW
		);
		expect(deviceSummary(rows)).toEqual({
			total: 3,
			checkingIn: 1,
			quiet: 1,
			signedOut: 1,
			wipesOutstanding: 1,
			// The quiet machine is still a current one: it is not signed out, so
			// it is a machine waiting to be heard from rather than one to leave
			// out of the count.
			current: 2,
			needingUpdateForSourcedFiles: 0
		});
	});

	it('lists only the marketplaces a machine currently holds', () => {
		const rows = deviceRows(
			[device({ sessions: [session('Tpt'), session('Tes', 'wiped')] })],
			NOW
		);
		expect(rows[0].holding).toEqual(['Tpt']);
	});
});

describe('where each login lives', () => {
	it('names the machine holding a no-API marketplace, and D30 words it', () => {
		const tpt = of(signInStates([device({ sessions: [session('Tpt')] })], [], NOW), 'Tpt');
		expect(tpt.state).toBe('signed_in');
		expect(tpt.device?.name).toBe('staffroom-laptop');
		expect(tpt.accountLabel).toBe('Miss Cooper');
		expect(tpt.line).toContain('never leaves that device');
	});

	it('says a schedule is not running when the holder has gone quiet', () => {
		const tpt = of(
			signInStates(
				[device({ last_seen_at: NOW - QUIET_AFTER_MS - 1, sessions: [session('Tpt')] })],
				[],
				NOW
			),
			'Tpt'
		);
		expect(tpt.state).toBe('signed_in');
		expect(tpt.tone).toBe('run');
		expect(tpt.line).toContain('not running');
	});

	it('separates having no machine at all from needing a sign-in on one', () => {
		const none = signInStates([], [], NOW);
		expect(none.every((entry) => entry.state !== 'needs_signin')).toBe(true);
		expect(of(none, 'Tpt').state).toBe('no_device');

		const some = signInStates([device({ sessions: [session('Tpt')] })], [], NOW);
		expect(of(some, 'Tes').state).toBe('needs_signin');
	});

	it('separates a registry emptied by signing out from one that was never filled', () => {
		const states = signInStates(
			[device({ revoked_at: NOW - 5, sessions: [session('Tpt')] })],
			[],
			NOW
		);
		const tpt = of(states, 'Tpt');
		expect(tpt.state).toBe('all_signed_out');
		expect(tpt.line).toContain('Every machine of yours is signed out');
		expect(needingDeviceSignIn(states)).toEqual([]);
	});

	it('names every marketplace whatever the order is ranked as', () => {
		const states = signInStates([], [], NOW);
		expect([...states].map((entry) => entry.marketplace).sort()).toEqual(['Etsy', 'Tes', 'Tpt']);
	});

	it('never asks for a device for a marketplace with a sanctioned API', () => {
		const etsy = of(signInStates([], [connection('Etsy')], NOW), 'Etsy');
		expect(etsy.state).toBe('served_here');
		expect(etsy.line).toContain('No device is needed');
	});

	it('does not call an unverified server-side connection served', () => {
		for (const status of ['checking', 'unstable'] as const) {
			const etsy = of(signInStates([], [connection('Etsy', status)], NOW), 'Etsy');
			expect(etsy.state).toBe('unverified');
			expect(etsy.line).toBe(present(status).explanation);
			expect(etsy.tone).toBe(present(status).tone);
		}
	});

	it('still reports a dropped server-side connection, in the connections page words', () => {
		const etsy = of(signInStates([], [connection('Etsy', 'disconnected')], NOW), 'Etsy');
		expect(etsy.state).toBe('needs_signin');
		expect(etsy.tone).toBe('bad');
		expect(etsy.line).toBe(present('disconnected').explanation);
	});

	it('separates a marketplace with no account linked from one that broke', () => {
		const etsy = of(signInStates([], [], NOW), 'Etsy');
		expect(etsy.state).toBe('no_account');
		expect(needingDeviceSignIn(signInStates([], [], NOW))).toEqual([]);
	});

	it('ignores a signed-out machine when deciding who holds a login', () => {
		const states = signInStates(
			[
				device({ id: 'gone', revoked_at: NOW - 5, sessions: [session('Tpt')] }),
				device({ id: 'here', sessions: [session('Tes')] })
			],
			[],
			NOW
		);
		expect(of(states, 'Tpt').state).toBe('needs_signin');
		expect(of(states, 'Tes').state).toBe('signed_in');
	});

	it('leaves a dropped server-side connection to the connections page', () => {
		const waiting = needingDeviceSignIn(
			signInStates(
				[device({ sessions: [session('Tpt'), session('Tes')] })],
				[connection('Etsy', 'disconnected')],
				NOW
			)
		);
		expect(waiting).toEqual([]);
	});

	it('collects what is waiting on a sign-in', () => {
		const waiting = needingDeviceSignIn(
			signInStates([device({ sessions: [session('Tpt')] })], [connection('Etsy')], NOW)
		);
		expect(waiting.map((entry) => entry.marketplace)).toEqual(['Tes']);
	});
});

describe('whether anything scheduled can run', () => {
	it('needs a machine that is both checking in and holding a login', () => {
		expect(schedulesRunning(deviceRows([device({ sessions: [session('Tpt')] })], NOW))).toBe(true);
		expect(schedulesRunning(deviceRows([device()], NOW))).toBe(false);
		expect(
			schedulesRunning(
				deviceRows(
					[device({ last_seen_at: NOW - QUIET_AFTER_MS - 1, sessions: [session('Tpt')] })],
					NOW
				)
			)
		).toBe(false);
	});
});

describe('what the band says when nothing is running', () => {
	function summaryOf(devices: DeviceView[]) {
		const rows = deviceRows(devices, NOW);
		return { summary: deviceSummary(rows), running: schedulesRunning(rows) };
	}

	it('says nothing at all when there is no machine to speak about', () => {
		const { summary, running } = summaryOf([]);
		expect(bandNotice(summary, running)).toBeNull();
	});

	it('says nothing while a machine is checking in and holding a login', () => {
		const { summary, running } = summaryOf([device({ sessions: [session('Tpt')] })]);
		expect(bandNotice(summary, running)).toBeNull();
	});

	it('does not blame silence for a registry the seller signed out', () => {
		const { summary, running } = summaryOf([
			device({ revoked_at: NOW - 5, sessions: [session('Tpt')] })
		]);
		const notice = bandNotice(summary, running);
		expect(notice?.kind).toBe('all_signed_out');
		expect(notice?.body).not.toContain('checked in');
	});

	it('blames silence only where a live machine has actually gone quiet', () => {
		const { summary, running } = summaryOf([
			device({ last_seen_at: NOW - QUIET_AFTER_MS - 1, sessions: [session('Tpt')] })
		]);
		const notice = bandNotice(summary, running);
		expect(notice?.kind).toBe('nothing_checking_in');
		expect(notice?.body).toContain('checked in for half an hour');
	});

	it('says a current machine has nothing to run when it holds no login', () => {
		const { summary, running } = summaryOf([device()]);
		const notice = bandNotice(summary, running);
		expect(notice?.kind).toBe('no_login');
		expect(notice?.body).toContain('nothing for it to run');
	});

	it('prefers the signed-out sentence over silence when both could be said', () => {
		const { summary, running } = summaryOf([
			device({ id: 'a', revoked_at: NOW - 5, last_seen_at: NOW - QUIET_AFTER_MS - 1 }),
			device({ id: 'b', revoked_at: NOW - 5, last_seen_at: NOW - QUIET_AFTER_MS - 1 })
		]);
		expect(bandNotice(summary, running)?.kind).toBe('all_signed_out');
	});
});

describe('the footnote under the marketplace rows', () => {
	function summaryOf(devices: DeviceView[]) {
		return deviceSummary(deviceRows(devices, NOW));
	}

	it('says nothing where there is no machine to count', () => {
		expect(deviceFootnote(summaryOf([]))).toBe('');
	});

	it('counts against the machines that could check in, not the ones signed out', () => {
		const line = deviceFootnote(
			summaryOf([
				device({ id: 'a', sessions: [session('Tpt')] }),
				device({ id: 'b', revoked_at: NOW - 5 })
			])
		);
		expect(line).toContain('1 of 1 machine has checked in');
		expect(line).toContain('1 other is signed out and kept in history.');
	});

	it('never divides by a registry that is entirely signed out', () => {
		const line = deviceFootnote(summaryOf([device({ revoked_at: NOW - 5 })]));
		expect(line).toBe('Every machine you have registered is signed out.');
		expect(line).not.toContain('0 of');
	});

	it('does not repeat the quiet count the denominator already gives', () => {
		const line = deviceFootnote(
			summaryOf([
				device({ id: 'a', sessions: [session('Tpt')] }),
				device({ id: 'b', last_seen_at: NOW - QUIET_AFTER_MS - 1 })
			])
		);
		expect(line).toBe('1 of 2 machines have checked in in the last half hour.');
	});

	it('carries the outstanding wipe wherever it lands', () => {
		expect(
			deviceFootnote(summaryOf([device({ revoked_at: NOW - 5, wipe_outstanding: true })]))
		).toContain('may still hold the marketplace logins');
		expect(
			deviceFootnote(
				summaryOf([
					device({ id: 'a', sessions: [session('Tpt')] }),
					device({ id: 'b', revoked_at: NOW - 5, wipe_outstanding: true })
				])
			)
		).toContain('may still hold the marketplace logins');
	});
});


/** Looked up rather than destructured by position, for the same reason `of`
 *  is: the screen's order is a ranked map. */
function rowOf(rows: readonly MarketplaceRow[], marketplace: Marketplace): MarketplaceRow {
	const found = rows.find((entry) => entry.marketplace === marketplace);
	if (found === undefined) {
		throw new Error(`no row for ${marketplace}`);
	}
	return found;
}

describe('the merged marketplace rows', () => {
	it('give one row per marketplace even where nothing is linked and no device exists', () => {
		const rows = marketplaceRows([], [], NOW);
		expect(rows.map((row) => row.marketplace)).toEqual(['Tpt', 'Tes', 'Etsy']);
	});

	it('name the branch each marketplace runs on', () => {
		const rows = marketplaceRows([], [], NOW);
		expect(rowOf(rows, 'Tpt').transport).toBe('SellerDevice');
		expect(rowOf(rows, 'Tes').transport).toBe('SellerDevice');
		expect(rowOf(rows, 'Etsy').transport).toBe('OfficialApi');
	});

	it('carry the connection record only on the branch that can have one', () => {
		const rows = marketplaceRows(
			[device({ sessions: [session('Tpt')] })],
			[connection('Etsy')],
			NOW
		);
		expect(rowOf(rows, 'Etsy').connection?.id).toBe('c-Etsy');
		expect(rowOf(rows, 'Tpt').connection).toBeNull();
	});

	it('carry the declaration on the branch whose connection record they do not', () => {
		const rows = marketplaceRows(
			[device({ sessions: [session('Tpt')] })],
			[
				{
					...connection('Tpt'),
					authorship: { state: 'declared', name: 'A Teacher', attested_at: NOW - 1_000 }
				},
				{ ...connection('Tes'), authorship: { state: 'undeclared' } },
				connection('Etsy')
			],
			NOW
		);
		expect(rowOf(rows, 'Tpt').authorship).toEqual({
			state: 'declared',
			name: 'A Teacher',
			attested_at: NOW - 1_000
		});
		expect(rowOf(rows, 'Tpt').connection).toBeNull();
		expect(rowOf(rows, 'Tes').authorship).toEqual({ state: 'undeclared' });
		expect(rowOf(rows, 'Etsy').authorship).toBeUndefined();
	});

	it('leave the declaration undefined where no connection row exists for the marketplace', () => {
		const rows = marketplaceRows([device({ sessions: [session('Tpt')] })], [], NOW);
		expect(rowOf(rows, 'Tpt').authorship).toBeUndefined();
	});

	it('report a login held on a machine that has gone quiet', () => {
		const quiet = device({ last_seen_at: NOW - QUIET_AFTER_MS - 1, sessions: [session('Tes')] });
		const rows = marketplaceRows([quiet], [], NOW);
		expect(rowOf(rows, 'Tes').quiet).toBe(true);
		expect(rowOf(rows, 'Tpt').quiet).toBe(false);
	});

	it('does not call a login quiet while its machine is checking in', () => {
		const rows = marketplaceRows([device({ sessions: [session('Tes')] })], [], NOW);
		expect(rowOf(rows, 'Tes').quiet).toBe(false);
	});

	it('name the machines a signed-out login may still be sitting on', () => {
		const stale = device({
			id: 'd2',
			name: 'old-desktop',
			revoked_at: NOW - 5_000,
			wipe_outstanding: true,
			sessions: [session('Tpt')]
		});
		const rows = marketplaceRows([stale], [], NOW);
		expect(rowOf(rows, 'Tpt').wipeOutstandingOn.map((entry) => entry.name)).toEqual([
			'old-desktop'
		]);
		expect(rowOf(rows, 'Tes').wipeOutstandingOn).toEqual([]);
	});

	it('leaves a machine that was signed out and has since checked in off that list', () => {
		const wiped = device({
			revoked_at: NOW - 5_000,
			wipe_outstanding: false,
			sessions: [session('Tpt')]
		});
		expect(rowOf(marketplaceRows([wiped], [], NOW), 'Tpt').wipeOutstandingOn).toEqual([]);
	});
});

describe('the marketplaces the screen puts at the top', () => {
	it('name a device-branch marketplace no machine holds', () => {
		const rows = marketplaceRows([device({ sessions: [session('Tpt')] })], [], NOW);
		expect(needingAttention(rows).map((row) => row.marketplace)).toEqual(['Tes']);
	});

	it('name a dropped connection on the branch we serve ourselves', () => {
		const rows = marketplaceRows(
			[device({ sessions: [session('Tpt'), session('Tes')] })],
			[connection('Etsy', 'disconnected')],
			NOW
		);
		expect(needingAttention(rows).map((row) => row.marketplace)).toEqual(['Etsy']);
	});

	it('name a connection that is verifying and failing, but not one merely unverified', () => {
		const held = [device({ sessions: [session('Tpt'), session('Tes')] })];
		expect(
			needingAttention(marketplaceRows(held, [connection('Etsy', 'unstable')], NOW))
		).toHaveLength(1);
		expect(
			needingAttention(marketplaceRows(held, [connection('Etsy', 'checking')], NOW))
		).toEqual([]);
	});

	it('name a login whose machine has gone quiet, because its schedule is not running', () => {
		const quiet = device({
			last_seen_at: NOW - QUIET_AFTER_MS - 1,
			sessions: [session('Tpt'), session('Tes')]
		});
		expect(needingAttention(marketplaceRows([quiet], [], NOW)).map((row) => row.marketplace)).toEqual(
			['Tpt', 'Tes']
		);
	});

	it('says nothing about a marketplace never linked or a seller with no machine', () => {
		expect(needingAttention(marketplaceRows([], [], NOW))).toEqual([]);
	});

	it('is empty where every marketplace is held and connected', () => {
		const rows = marketplaceRows(
			[device({ sessions: [session('Tpt'), session('Tes')] })],
			[connection('Etsy')],
			NOW
		);
		expect(needingAttention(rows)).toEqual([]);
	});
});

describe('readiness, as facts that are kept apart', () => {
	// The constraint the founder's report turns on: a credential saved on a
	// machine is not the machine being there. A machine holding a TPT login
	// that has been off for hours runs nothing, and the page must not imply
	// otherwise.
	it('never infers presence from a saved marketplace login', () => {
		const rows = deviceRows(
			[device({ last_seen_at: NOW - QUIET_AFTER_MS - 1, sessions: [session('Tpt')] })],
			NOW
		);
		expect(rows[0].standing).toBe('quiet');
		expect(rows[0].holding).toEqual(['Tpt']);
		expect(schedulesRunning(rows)).toBe(false);
	});

	// And the version floor is not permission to work. The import claim admits
	// any registered, unrevoked device without looking at a version, and the
	// floor applies only to an item whose payload comes from a marketplace. So
	// an older installation that is here and holds a login is running the
	// schedule, and saying otherwise would report a working fleet as stalled.
	it('does not treat the sourced-file floor as permission to work', () => {
		const rows = deviceRows(
			[device({ app_version: '0.8.0', runs_sourced_payloads: false, sessions: [session('Tpt')] })],
			NOW
		);
		expect(rows[0].standing).toBe('checking_in');
		expect(rows[0].runsSourcedFiles).toBe(false);
		expect(schedulesRunning(rows)).toBe(true);
		const summary = deviceSummary(rows);
		expect(summary.current).toBe(1);
		expect(summary.needingUpdateForSourcedFiles).toBe(1);
		// Nothing is claimed to be stalled, and the narrow limitation is named
		// as narrow.
		expect(bandNotice(summary, schedulesRunning(rows))).toBeNull();
		const line = deviceFootnote(summary);
		expect(line).toContain('publish or refetch a file from a marketplace');
		expect(line).toContain('Everything else runs there as normal');
	});

	it('counts a current machine and runs the schedule on it', () => {
		const rows = deviceRows([device({ sessions: [session('Tpt')] })], NOW);
		expect(deviceSummary(rows).current).toBe(1);
		expect(schedulesRunning(rows)).toBe(true);
	});

	// Counts are about what the page shows. A seller with one working laptop
	// and three replaced installations read "1 of 4" and could see one row.
	it('counts the current machines and keeps signed-out records out of the total it divides by', () => {
		const rows = deviceRows(
			[
				device({ id: 'live', sessions: [session('Tpt')] }),
				device({ id: 'old-1', revoked_at: NOW - 5 }),
				device({ id: 'old-2', revoked_at: NOW - 6 }),
				device({ id: 'old-3', revoked_at: NOW - 7 })
			],
			NOW
		);
		const summary = deviceSummary(rows);
		expect(summary.current).toBe(1);
		expect(summary.signedOut).toBe(3);
		expect(deviceFootnote(summary)).toContain('1 of 1 machine has checked in');
	});
});
