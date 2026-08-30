// The connections page's rendering of the four-state status: a tone and a
// sentence per value. Pure, so it tests without a component.
//
// The four labels are the server's own words, rendered verbatim rather than
// prettified: the vocabulary is generated from the Rust enum, and a client
// that re-spells it puts a second vocabulary in front of the seller.

import type { ConnectionStatus } from '$lib/generated/vocab';

export interface StatusPresentation {
	label: ConnectionStatus;
	/// The pill modifier the design system renders this status under.
	tone: 'ok' | 'mut' | 'run' | 'bad';
	/// What the value means for the seller, in one sentence.
	explanation: string;
}

const PRESENTATION: Record<ConnectionStatus, Omit<StatusPresentation, 'label'>> = {
	connected: {
		tone: 'ok',
		explanation: 'Verified against the marketplace recently. Work is flowing.'
	},
	checking: {
		tone: 'mut',
		explanation: 'Linked, but not verified right now. Nothing for you to do.'
	},
	unstable: {
		tone: 'run',
		explanation:
			'Verification is failing. We are still retrying, and re-linking is not known to be the fix.'
	},
	disconnected: {
		tone: 'bad',
		explanation: 'Nothing usable is stored. Re-link to let queued work continue.'
	}
};

export function present(status: ConnectionStatus): StatusPresentation {
	return { label: status, ...PRESENTATION[status] };
}
