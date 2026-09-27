// Whether the guided tour is open and on which step. A module store rather
// than state in the shell, because two places open it: the shell, when the
// profile says it is due, and "Show me around" on Help and guides.

import { backIndex, nextIndex, TOUR_STEPS } from './model';

class TourStore {
	open = $state(false);
	index = $state(0);

	/** Opens the tour on its first step, whatever the profile says. */
	start() {
		this.index = 0;
		this.open = true;
	}

	next() {
		this.index = nextIndex(this.index);
	}

	back() {
		this.index = backIndex(this.index);
	}

	get last(): boolean {
		return this.index === TOUR_STEPS.length - 1;
	}

	close() {
		this.open = false;
	}
}

export const tour = new TourStore();
