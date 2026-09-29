import { describe, expect, it } from 'vitest';
import {
	backIndex,
	DOCK_BELOW,
	GAP,
	nextIndex,
	PAD,
	placeTour,
	GET_THE_APP_HREF,
	shouldOfferTour,
	tourSteps,
	visibleRect,
	whichTour,
	type Rect
} from './model';

const DESKTOP = { width: 1280, height: 800 };
const PHONE = { width: 390, height: 844 };
const CARD = { width: 340, height: 180 };

describe('whether the shell offers the tour by itself', () => {
	const plain = { impersonating: false, checkoutSuccess: false };

	it('offers it to a new account, and again on checkout', () => {
		expect(shouldOfferTour({ ...plain, tour: 'due' })).toBe(true);
		expect(shouldOfferTour({ ...plain, tour: 'due', checkoutSuccess: true })).toBe(true);
	});

	it('never offers it again once it was completed or skipped, even after a purchase', () => {
		for (const tour of ['completed', 'skipped'] as const) {
			expect(shouldOfferTour({ ...plain, tour })).toBe(false);
			expect(shouldOfferTour({ ...plain, tour, checkoutSuccess: true })).toBe(false);
		}
	});

	it('offers it to an account older than the tour only on the return from buying a plan', () => {
		expect(shouldOfferTour({ ...plain, tour: 'predates' })).toBe(false);
		expect(shouldOfferTour({ ...plain, tour: 'predates', checkoutSuccess: true })).toBe(true);
	});

	it('never offers it to an operator impersonating, whatever the user is due', () => {
		for (const tour of ['due', 'predates'] as const) {
			expect(shouldOfferTour({ tour, impersonating: true, checkoutSuccess: true })).toBe(false);
		}
	});

	it('waits for the profile rather than guessing', () => {
		expect(shouldOfferTour({ ...plain, tour: undefined, checkoutSuccess: true })).toBe(false);
	});
});

describe('which tour the shell opens by itself', () => {
	const due = { tour: 'due', impersonating: false, checkoutSuccess: false } as const;

	it('opens the app tour on the first open of the app on a device, ahead of the console tour', () => {
		expect(whichTour({ ...due, host: 'app', appTourSeen: false })).toBe('app-first-open');
		expect(whichTour({ ...due, tour: 'completed', host: 'app', appTourSeen: false })).toBe(
			'app-first-open'
		);
	});

	it('opens the console tour, if due, once this device has shown the app tour', () => {
		expect(whichTour({ ...due, host: 'app', appTourSeen: true })).toBe('console');
		expect(whichTour({ ...due, tour: 'skipped', host: 'app', appTourSeen: true })).toBeNull();
	});

	it('never opens the app tour in a browser, whatever this device has seen', () => {
		expect(whichTour({ ...due, host: 'browser', appTourSeen: false })).toBe('console');
		expect(
			whichTour({ ...due, tour: 'completed', host: 'browser', appTourSeen: false })
		).toBeNull();
	});

	it('opens neither for an operator impersonating', () => {
		expect(whichTour({ ...due, impersonating: true, host: 'app', appTourSeen: false })).toBeNull();
	});
});

describe('the steps', () => {
	it('walks the sections, then the resource, the app, billing, and ends on a card of its own', () => {
		for (const host of ['app', 'browser'] as const) {
			expect(tourSteps('console', host).map((step) => step.anchor)).toEqual([
				'import',
				'crosslist',
				'new',
				'automations',
				'marketplaces',
				'billing',
				null
			]);
		}
	});

	it('ends a browser tour on getting the app, and an app tour on the first resource', () => {
		const browser = tourSteps('console', 'browser').at(-1);
		expect(browser?.title).toBe('Start here: download the app');
		expect(browser?.action?.href).toBe(GET_THE_APP_HREF);
		expect(GET_THE_APP_HREF).toBe('/marketplaces#step-app');

		const app = tourSteps('console', 'app').at(-1);
		expect(app?.title).toBe('Start here');
		expect(app?.action).toMatchObject({
			label: 'Create your first resource',
			href: '/resources/new'
		});
	});

	it('keeps the app first-open tour short and ends it at Marketplaces', () => {
		const steps = tourSteps('app-first-open', 'app');
		expect(steps.length).toBeGreaterThanOrEqual(2);
		expect(steps.length).toBeLessThanOrEqual(3);
		expect(steps.some((step) => step.anchor === 'marketplaces')).toBe(true);
		expect(steps.at(-1)?.action?.href).toBe('/marketplaces');
	});

	it('only the last step of each tour carries a button of its own', () => {
		for (const steps of [
			tourSteps('console', 'app'),
			tourSteps('console', 'browser'),
			tourSteps('app-first-open', 'app')
		]) {
			expect(steps.map((step) => step.action !== undefined)).toEqual(
				steps.map((_, index) => index === steps.length - 1)
			);
		}
	});

	it('holds at either end rather than running off it', () => {
		expect(backIndex(0)).toBe(0);
		expect(nextIndex(0, 3)).toBe(1);
		expect(nextIndex(2, 3)).toBe(2);
		expect(backIndex(2)).toBe(1);
	});
});

describe('which of several elements carrying a key is lit', () => {
	const hidden: Rect = { top: 0, left: 0, width: 0, height: 0 };
	const rail: Rect = { top: 80, left: 10, width: 36, height: 36 };
	const offscreen: Rect = { top: 900, left: 10, width: 36, height: 36 };

	it('skips the copy the stylesheet hides and the one outside the window', () => {
		expect(visibleRect([hidden, offscreen, rail], DESKTOP)).toEqual(rail);
	});

	it('is none where no copy is on screen', () => {
		expect(visibleRect([hidden, offscreen], DESKTOP)).toBeNull();
		expect(visibleRect([], DESKTOP)).toBeNull();
	});
});

describe('where the spotlight and the card go', () => {
	it('pads the spotlight around the element', () => {
		const target = { top: 100, left: 10, width: 36, height: 36 };
		expect(placeTour(target, DESKTOP, CARD).spotlight).toEqual({
			top: 100 - PAD,
			left: 10 - PAD,
			width: 36 + PAD * 2,
			height: 36 + PAD * 2
		});
	});

	it('floats the card to the right of a rail element on a desktop', () => {
		const { card } = placeTour({ top: 100, left: 10, width: 36, height: 36 }, DESKTOP, CARD);
		expect(card).toEqual({ kind: 'floating', top: 100 - PAD, left: 46 + PAD + GAP, width: 340 });
	});

	it('keeps a card beside a low element inside the window', () => {
		const { card } = placeTour({ top: 760, left: 10, width: 36, height: 36 }, DESKTOP, CARD);
		expect(card.kind === 'floating' && card.top + CARD.height).toBeLessThanOrEqual(
			DESKTOP.height - GAP
		);
	});

	it('puts the card below an element at the right edge, where it cannot sit beside it', () => {
		const target = { top: 16, left: 1100, width: 160, height: 38 };
		const { card } = placeTour(target, DESKTOP, CARD);
		expect(card.kind).toBe('floating');
		if (card.kind === 'floating') {
			expect(card.top).toBe(16 + 38 + PAD + GAP);
			expect(card.left + card.width).toBeLessThanOrEqual(DESKTOP.width - GAP);
		}
	});

	it('centres a card with nothing to light', () => {
		const { spotlight, card } = placeTour(null, DESKTOP, CARD);
		expect(spotlight).toBeNull();
		expect(card).toEqual({
			kind: 'floating',
			top: (DESKTOP.height - CARD.height) / 2,
			left: (DESKTOP.width - 340) / 2,
			width: 340
		});
	});

	it('docks the card to the bottom edge on a phone', () => {
		const header = { top: 20, left: 330, width: 40, height: 40 };
		expect(placeTour(header, PHONE, CARD).card).toEqual({ kind: 'docked', bottom: 0 });
		expect(placeTour(null, PHONE, CARD).card).toEqual({ kind: 'docked', bottom: 0 });
	});

	it('stands the docked card on top of a phone-bar element rather than covering it', () => {
		const tab = { top: 780, left: 0, width: 78, height: 64 };
		const { spotlight, card } = placeTour(tab, PHONE, CARD);
		expect(card).toEqual({ kind: 'docked', bottom: PHONE.height - (780 - PAD) + GAP });
		expect(spotlight?.top).toBe(780 - PAD);
	});

	it('docks at the console’s own phone breakpoint and floats from it', () => {
		const target = { top: 100, left: 10, width: 36, height: 36 };
		expect(placeTour(target, { width: DOCK_BELOW - 1, height: 800 }, CARD).card.kind).toBe(
			'docked'
		);
		expect(placeTour(target, { width: DOCK_BELOW, height: 800 }, CARD).card.kind).toBe('floating');
	});
});
