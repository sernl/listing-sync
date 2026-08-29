// The connections page's rendering of the four-state status: a tone and a
// sentence per value. Pure, so it tests without a component.
//
// The four labels are the server's own words, rendered verbatim rather than
// prettified: the vocabulary is generated from the Rust enum, and a client
// that re-spells it puts a second vocabulary in front of the seller.

import type { ConnectionStatus } from '$lib/generated/vocab';

export interface StatusPresentation {
	label: ConnectionStatus;
	tone: string;
	/// What the value means for the seller, in one sentence.
	explanation: string;
}

const PRESENTATION: Record<ConnectionStatus, Omit<StatusPresentation, 'label'>> = {
	connected: {
		tone: 'bg-emerald-100 text-emerald-800',
		explanation: 'Verified against the marketplace recently. Work is flowing.'
	},
	checking: {
		tone: 'bg-slate-100 text-slate-700',
		explanation: 'Linked, but not verified right now. Nothing for you to do.'
	},
	unstable: {
		tone: 'bg-amber-100 text-amber-900',
		explanation:
			'Verification is failing. We are still retrying, and re-linking is not known to be the fix.'
	},
	disconnected: {
		tone: 'bg-orange-100 text-orange-800',
		explanation: 'Nothing usable is stored. Re-link to let queued work continue.'
	}
};

export function present(status: ConnectionStatus): StatusPresentation {
	return { label: status, ...PRESENTATION[status] };
}
