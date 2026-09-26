import { describe, expect, it } from 'vitest';
import {
	NO_PROBE,
	claimBlockedReason,
	claimStatus,
	consoleGate,
	worthProbing,
	type Probe
} from './state';

const settled = (slug: string, available: boolean): Probe => ({
	slug,
	pending: false,
	available,
	failed: false
});

describe('what the claim screen says about the slug in the field', () => {
	it('says nothing at all about an empty field', () => {
		expect(claimStatus('', NO_PROBE)).toEqual({ kind: 'empty' });
		expect(claimStatus('   ', NO_PROBE)).toEqual({ kind: 'empty' });
	});

	it('states the shape refusal without waiting for a probe', () => {
		const status = claimStatus('ab_c', NO_PROBE);
		expect(status.kind).toBe('invalid');
		expect(status.kind === 'invalid' && status.message).toBe(
			'Use letters, numbers and hyphens only.'
		);
	});

	it('is checking while nothing has judged the value now in the field', () => {
		expect(claimStatus('riverbend', NO_PROBE)).toEqual({ kind: 'checking', slug: 'riverbend' });
	});

	it('does not render an older keystroke as a verdict on this one', () => {
		expect(claimStatus('riverbend-two', settled('riverbend', true))).toEqual({
			kind: 'checking',
			slug: 'riverbend-two'
		});
	});

	it('is checking while a probe for this value is still in flight', () => {
		expect(
			claimStatus('riverbend', { slug: 'riverbend', pending: true, available: null, failed: false })
		).toEqual({ kind: 'checking', slug: 'riverbend' });
	});

	it('reports a free name as available', () => {
		const status = claimStatus('  RiverBend  ', settled('riverbend', true));
		expect(status.kind).toBe('available');
		// The normalised form, so the seller sees what they would get rather
		// than what they typed.
		expect(status.kind === 'available' && status.slug).toBe('riverbend');
	});

	it('reports a held name as taken, in the seller’s words', () => {
		const status = claimStatus('riverbend', settled('riverbend', false));
		expect(status.kind).toBe('taken');
		expect(status.kind === 'taken' && status.message).toBe('That name is taken. Try another.');
	});

	// The defect this separates: a probe that failed and a name someone else
	// holds are different facts, and folding the first into the second tells a
	// seller their chosen name is taken because our own request failed.
	it('keeps a probe that failed apart from a name that is taken', () => {
		const failed = claimStatus('riverbend', {
			slug: 'riverbend',
			pending: false,
			available: null,
			failed: true
		});
		expect(failed.kind).toBe('unknown');
		expect(failed).not.toEqual(claimStatus('riverbend', settled('riverbend', false)));
	});
});

describe('when the claim can be submitted', () => {
	it('blocks on an empty, malformed or taken name and says why', () => {
		expect(claimBlockedReason(claimStatus('', NO_PROBE), false)).toBe(
			'Choose a name for your account.'
		);
		expect(claimBlockedReason(claimStatus('ab_c', NO_PROBE), false)).toBe(
			'Use letters, numbers and hyphens only.'
		);
		expect(claimBlockedReason(claimStatus('riverbend', settled('riverbend', false)), false)).toBe(
			'That name is taken. Try another.'
		);
	});

	it('allows a name the probe judged free', () => {
		expect(claimBlockedReason(claimStatus('riverbend', settled('riverbend', true)), false)).toBe(
			null
		);
	});

	// The probe is advisory and the server's index is the arbiter, so a seller
	// whose one request dropped is not stranded on a name they could have had.
	it('allows a name the probe could not judge, and one still being checked', () => {
		expect(
			claimBlockedReason(
				claimStatus('riverbend', { slug: 'riverbend', pending: false, available: null, failed: true }),
				false
			)
		).toBe(null);
		expect(claimBlockedReason(claimStatus('riverbend', NO_PROBE), false)).toBe(null);
	});

	it('blocks while the claim is being saved, whatever the status', () => {
		expect(claimBlockedReason(claimStatus('riverbend', settled('riverbend', true)), true)).toBe(
			'Saving your name.'
		);
	});
});

describe('which drafts are worth an availability probe', () => {
	it('probes only a slug the client would accept, and probes the normalised form', () => {
		expect(worthProbing('  RiverBend  ')).toBe('riverbend');
		expect(worthProbing('ab')).toBe(null);
		expect(worthProbing('ab_c')).toBe(null);
		expect(worthProbing('admin')).toBe(null);
		expect(worthProbing('')).toBe(null);
	});
});

describe('what the console shell does about an unclaimed slug', () => {
	const gate = (over: Partial<Parameters<typeof consoleGate>[0]> = {}) =>
		consoleGate({
			prompt: 'claim',
			impersonating: false,
			publicRoute: false,
			bannerDismissed: false,
			...over
		});

	it('puts the claim screen in front of a newly provisioned organisation', () => {
		expect(gate()).toBe('claim-screen');
	});

	it('shows a dismissable banner to an organisation that predates the slug', () => {
		expect(gate({ prompt: 'banner' })).toBe('banner');
		expect(gate({ prompt: 'banner', bannerDismissed: true })).toBe('nothing');
	});

	it('asks nothing of an organisation that has claimed one', () => {
		expect(gate({ prompt: 'settled' })).toBe('nothing');
	});

	// The defect this exists for: the claim screen renders outside `Console`,
	// and `Console` carries the stop-impersonating control. Gating an
	// impersonated session therefore strands the operator inside the tenant
	// with no way back out -- the one action they need is the one the gate
	// takes away. The tenant names their own organisation; the operator is
	// only visiting.
	it('never gates an operator who is impersonating, so the way out survives', () => {
		expect(gate({ impersonating: true })).toBe('banner');
		expect(gate({ impersonating: true, bannerDismissed: true })).toBe('nothing');
	});

	it('gates the same session once the impersonation ends', () => {
		expect(gate({ impersonating: true })).not.toBe('claim-screen');
		expect(gate({ impersonating: false })).toBe('claim-screen');
	});

	it('leaves the public pages outside the gate', () => {
		// `/status` matters most when signing in is what is broken, so a seller
		// kept off it for not having named their organisation is kept from the
		// page that would explain why.
		expect(gate({ publicRoute: true })).toBe('nothing');
		expect(gate({ prompt: 'banner', publicRoute: true })).toBe('nothing');
	});

	it('asks nothing before the session has been read', () => {
		// Rendering a prompt from an unread session is a claim about a row
		// nobody has looked at.
		expect(gate({ prompt: undefined })).toBe('nothing');
	});
});
