import { describe, expect, it } from 'vitest';
import { present } from './connection-status';
import type { ConnectionStatus } from '$lib/generated/vocab';

const ALL: ConnectionStatus[] = ['connected', 'checking', 'unstable', 'disconnected'];

describe('the connection status chip', () => {
	it('renders the four words verbatim', () => {
		expect(ALL.map((status) => present(status).label)).toEqual([
			'connected',
			'checking',
			'unstable',
			'disconnected'
		]);
	});

	it('gives every status its own tone, so two never read alike', () => {
		const tones = ALL.map((status) => present(status).tone);
		expect(new Set(tones).size).toBe(ALL.length);
	});

	it('says what each one means without asking for action where none is due', () => {
		expect(present('checking').explanation).toMatch(/nothing for you to do/i);
		expect(present('unstable').explanation).toMatch(/may not be the fix/i);
		expect(present('disconnected').explanation).toMatch(/sign in again/i);
	});
});
