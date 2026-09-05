// The Marketplace Sharing page's settings, laid out but not wired.
//
// There is no share, bump, follower or relist concept anywhere behind this
// console, and neither TES nor TPT has a captured endpoint for one. D1 puts
// any such request on the seller's own machine under the seller's own session,
// so this is device work and an adapter flow before it is a console page. The
// shape is here, disabled, so the founder can correct it before anything is
// written against it. Pure, so it tests without a component.

export const NOT_BUILT_TITLE = 'Scheduled sharing is not built yet';

export const NOT_BUILT_BODY =
	'This page shows what it will do. Nothing here reaches a marketplace yet.';

export const NO_DEVICE_TITLE = 'Sharing runs on your own computer';

export const NO_DEVICE_BODY =
	'Install the Teachouse app to give the schedule a computer to run on. Your marketplace ' +
	'sign-in stays there.';

/** What the settings card is, stated on the card itself rather than only in
 *  the banner above it: nothing here is a setting in force, because no route
 *  stores one. Without this line a control's position reads as the current
 *  value of something. */
export const SETTINGS_ARE_A_PREVIEW =
	'What sharing would offer here once it is built. None of these is switched on, and ' +
	'nothing here is saved.';

/** Why every control on this page is disabled. Every disabled control in this
 *  console states its reason, because one that does not reads as a fault. */
export const DISABLED_REASON = 'Scheduled sharing is not built yet.';

/** No Automations page carries a button that runs anything now. D1 puts the
 *  timer on the seller's device and states that the server never says now, so
 *  a control that appeared to start a marketplace request immediately would be
 *  a promise the architecture forbids. Every page schedules, and says so. */
export const WHEN_OPTIONS = ['Every day', 'Every weekday', 'Every Monday', 'Off'] as const;
export type When = (typeof WHEN_OPTIONS)[number];

/** The seller's own timezone, named beneath the time field so a schedule is
 *  never ambiguous about which clock it means. */
export function timezoneName(): string {
	const zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
	return typeof zone === 'string' && zone.length > 0
		? zone
		: 'the time on this computer';
}

/** What the timezone line reads in full. */
export function timezoneLine(zone: string): string {
	return `Times are ${zone}.`;
}
