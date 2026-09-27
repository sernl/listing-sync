// The guided tour's own model: its steps, the rule for offering it, and
// where the spotlight and the card go on the screen. Pure, so it tests
// without a browser; `Tour.svelte` measures the page and draws the answer.
//
// The tour points at the console's own shell rather than at pages, so it
// never navigates: every step's element is on every signed-in page. Each
// step names a `data-tour` key rather than a selector, and an element carries
// the key it answers for. The desktop rail and the phone bar both carry the
// section keys, and whichever one is on screen is the one lit.

import type { TourState } from '$lib/api';

export interface TourStep {
	/** The `data-tour` key of the element this step lights, or `null` for a
	 *  step that is a card alone. */
	anchor: string | null;
	title: string;
	/** One sentence, in the founder's voice. */
	body: string;
}

/** The founder's order: the sections as the phone bar lists them, what a
 *  resource is, the app and the connections, the plan, then the first thing
 *  to do. Marketplaces is one step rather than a tab step and an app step,
 *  because both sentences are about the same element. */
export const TOUR_STEPS: readonly TourStep[] = [
	{
		anchor: 'import',
		title: 'Import',
		body: 'Bring in the resources you already sell, from wherever they live now.'
	},
	{
		anchor: 'crosslist',
		title: 'Catalogue',
		body: 'Every resource you have is here, ready to publish anywhere you sell.'
	},
	{
		anchor: 'new',
		title: 'New resource',
		body: 'A resource is one thing you sell: its files, cover, description and price, written once for every marketplace.'
	},
	{
		anchor: 'automations',
		title: 'Automate',
		body: 'Copy, price and publish resources on a schedule, and keep every listing up to date.'
	},
	{
		anchor: 'marketplaces',
		title: 'Marketplaces',
		body: 'Get the Teachouse app on your computer, then connect each place you sell from it.'
	},
	{
		anchor: 'billing',
		title: 'Billing',
		body: 'Your plan, payments and invoices are under Account.'
	},
	{
		anchor: null,
		title: 'Start here',
		body: 'Create your first resource, and we will walk you through each part of it.'
	}
];

/** What the shell knows when it decides whether to offer the tour. */
export interface OfferContext {
	/** The profile's tour state, or `undefined` while the profile is unread. */
	tour: TourState | undefined;
	/** An operator is signed in as this user. */
	impersonating: boolean;
	/** This page load is the return from a successful plan checkout. */
	checkoutSuccess: boolean;
}

/** Whether the shell opens the tour by itself.
 *
 * A new account is offered it on every sign-in until it is completed or
 * skipped. An account that predates the tour is offered it once, on the
 * return from buying a plan, because that is when the console becomes
 * theirs to learn. Never to an operator impersonating: the tour is the
 * user's to end, and an operator ending it would end it for them. Never while
 * the profile is unread, so the tour cannot flash open for somebody who has
 * already finished it. */
export function shouldOfferTour({ tour, impersonating, checkoutSuccess }: OfferContext): boolean {
	if (impersonating || tour === undefined) {
		return false;
	}
	switch (tour) {
		case 'due':
			return true;
		case 'predates':
			return checkoutSuccess;
		case 'completed':
		case 'skipped':
			return false;
	}
}

/** The step after `index`, held at the last one. */
export function nextIndex(index: number): number {
	return Math.min(index + 1, TOUR_STEPS.length - 1);
}

/** The step before `index`, held at the first one. */
export function backIndex(index: number): number {
	return Math.max(index - 1, 0);
}

export interface Rect {
	top: number;
	left: number;
	width: number;
	height: number;
}

export interface Size {
	width: number;
	height: number;
}

/** The first candidate that is actually on screen.
 *
 * Several elements carry one key -- the rail's Import and the phone bar's
 * Import -- and the stylesheet hides all but one of them at any width. A
 * hidden element measures as an empty box, and one scrolled off the page
 * measures outside the viewport; neither is anything to point at. */
export function visibleRect(candidates: readonly Rect[], viewport: Size): Rect | null {
	return (
		candidates.find(
			(rect) =>
				rect.width > 0 &&
				rect.height > 0 &&
				rect.left < viewport.width &&
				rect.top < viewport.height &&
				rect.left + rect.width > 0 &&
				rect.top + rect.height > 0
		) ?? null
	);
}

/** Below this width the card docks to the bottom edge at full width, which
 *  is the console's own phone breakpoint: the rail gives way to the bar. */
export const DOCK_BELOW = 620;

/** Room kept between the spotlight and what it lights, and between the card
 *  and the viewport's edge. */
export const PAD = 8;
export const GAP = 12;

export type CardPlacement =
	| { kind: 'floating'; top: number; left: number; width: number }
	| { kind: 'docked'; bottom: number };

export interface Placement {
	/** The cut-out, padded around the element, or `null` for a card-only step
	 *  or an element that is not on screen. */
	spotlight: Rect | null;
	card: CardPlacement;
}

const CARD_WIDTH = 340;

function clamp(value: number, low: number, high: number): number {
	return Math.min(Math.max(value, low), Math.max(low, high));
}

/** Where the spotlight and the card go for one step.
 *
 * On a phone the card sits at the bottom edge, full width, which is where a
 * thumb is. The phone bar is also at the bottom edge, so where the lit
 * element would be under the card the card stands on top of the element
 * instead, still full width.
 *
 * Wider, the card floats beside the element: to its right first, because the
 * rail and the navigation card are on the left edge, then below, above, and
 * left. With nothing lit it sits in the middle of the screen. */
export function placeTour(target: Rect | null, viewport: Size, card: Size): Placement {
	const spotlight =
		target === null
			? null
			: {
					top: target.top - PAD,
					left: target.left - PAD,
					width: target.width + PAD * 2,
					height: target.height + PAD * 2
				};

	if (viewport.width < DOCK_BELOW) {
		if (spotlight !== null && spotlight.top + spotlight.height > viewport.height - card.height - GAP) {
			return { spotlight, card: { kind: 'docked', bottom: viewport.height - spotlight.top + GAP } };
		}
		return { spotlight, card: { kind: 'docked', bottom: 0 } };
	}

	const width = Math.min(CARD_WIDTH, viewport.width - GAP * 2);
	const maxLeft = viewport.width - width - GAP;
	const maxTop = viewport.height - card.height - GAP;

	if (spotlight === null) {
		return {
			spotlight,
			card: {
				kind: 'floating',
				top: clamp((viewport.height - card.height) / 2, GAP, maxTop),
				left: clamp((viewport.width - width) / 2, GAP, maxLeft),
				width
			}
		};
	}

	const right = spotlight.left + spotlight.width + GAP;
	const below = spotlight.top + spotlight.height + GAP;
	const above = spotlight.top - GAP - card.height;
	const left = spotlight.left - GAP - width;
	const alongside = clamp(spotlight.top, GAP, maxTop);
	const centred = clamp(spotlight.left + spotlight.width / 2 - width / 2, GAP, maxLeft);

	const floating = (top: number, leftEdge: number): Placement => ({
		spotlight,
		card: { kind: 'floating', top, left: leftEdge, width }
	});
	if (right + width <= viewport.width - GAP) {
		return floating(alongside, right);
	}
	if (below + card.height <= viewport.height - GAP) {
		return floating(below, centred);
	}
	if (above >= GAP) {
		return floating(above, centred);
	}
	if (left >= GAP) {
		return floating(alongside, left);
	}
	return floating(maxTop, centred);
}
