// Whether a guided tour is open, which one, and on which step. A module store
// rather than state in the shell, because two places open it: the shell, when
// a tour is due, and "Show me around" on Help and guides.

import { desktopInvoker } from '$lib/desktop';
import { backIndex, nextIndex, tourSteps, type TourKind, type TourStep } from './model';

/** Which host this console is in, read the way every other surface reads it:
 *  the application's invoker is there or it is not. */
export function tourHost(): 'app' | 'browser' {
	return desktopInvoker() === null ? 'browser' : 'app';
}

class TourStore {
	open = $state(false);
	index = $state(0);
	kind = $state<TourKind>('console');
	steps = $state<readonly TourStep[]>(tourSteps('console', 'browser'));

	/** Opens a tour on its first step, whatever the profile says. */
	start(kind: TourKind = 'console') {
		this.kind = kind;
		this.steps = tourSteps(kind, tourHost());
		this.index = 0;
		this.open = true;
	}

	next() {
		this.index = nextIndex(this.index, this.steps.length);
	}

	back() {
		this.index = backIndex(this.index);
	}

	get last(): boolean {
		return this.index === this.steps.length - 1;
	}

	close() {
		this.open = false;
	}
}

export const tour = new TourStore();
