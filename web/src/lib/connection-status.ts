// The connections page's rendering of the four-state status: a tone, a
// sentence, and the word the top strip raises. Pure, so it tests without a
// component.
//
// The four labels are the server's own words, rendered verbatim rather than
// prettified: the vocabulary is generated from the Rust enum, and a client
// that re-spells it puts a second vocabulary in front of the seller. The
// sentence beside them is not the server's, and is written for a teacher.

import type { ConnectionStatus } from '$lib/generated/vocab';

export interface StatusPresentation {
	label: ConnectionStatus;
	/// The pill modifier the design system renders this status under.
	tone: 'ok' | 'mut' | 'run' | 'bad';
	/// What the value means for the seller, in one sentence.
	explanation: string;
	/// What the top strip says after the marketplace's name, and null where
	/// there is nothing to raise: a working connection and an unverified one
	/// are both "nothing for you to do", and a strip that said so would be a
	/// permanent notice nobody reads.
	alert: string | null;
}

const PRESENTATION: Record<ConnectionStatus, Omit<StatusPresentation, 'label'>> = {
	connected: {
		tone: 'ok',
		explanation: 'Working. Nothing for you to do.',
		alert: null
	},
	checking: {
		tone: 'mut',
		explanation: 'Connected. Nothing for you to do.',
		alert: null
	},
	unstable: {
		tone: 'run',
		explanation:
			'We are having trouble reaching this marketplace. We keep trying; signing in again may not be the fix.',
		alert: 'needs a look'
	},
	disconnected: {
		tone: 'bad',
		explanation:
			'You are signed out. Sign in again on your computer so waiting items can go out.',
		alert: 'signed out'
	}
};

export function present(status: ConnectionStatus): StatusPresentation {
	return { label: status, ...PRESENTATION[status] };
}
