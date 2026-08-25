// A static single-page app: no server rendering, everything client-side,
// exactly what ServeDir's index fallback serves.
export const ssr = false;
export const prerender = false;

import { api, ApiFailure, type Whoami } from '$lib/api';

export async function load(): Promise<{ session: Whoami | null }> {
	try {
		return { session: await api.whoami() };
	} catch (failure) {
		if (failure instanceof ApiFailure && failure.status === 401) {
			return { session: null };
		}
		throw failure;
	}
}
