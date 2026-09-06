<script lang="ts">
	import { tick, type Snippet } from 'svelte';
	import { afterBannerDismissed, focusRegion } from '$lib/focus-return';
	import Icon from '$lib/Icon.svelte';
	import type { IconName } from '$lib/icons';

	type Tone = 'info' | 'warn' | 'bad' | 'ok';

	let {
		tone = 'info',
		title,
		children,
		action,
		onDismiss,
		dismissLabel
	}: {
		tone?: Tone;
		title?: string;
		children: Snippet;
		action?: Snippet;
		/** Close this banner. Given only where the page still explains itself
		 *  once the sentence is gone: a banner that is the whole account of an
		 *  empty page leaves a seller looking at what reads as an empty account.
		 *  Whether the dismissal outlives the visit is the caller's, since only
		 *  it knows whether the sentence is about the product or about state. */
		onDismiss?: () => void;
		/** What the close control is called, where `Dismiss: {title}` is wrong. */
		dismissLabel?: string;
	} = $props();

	const GLYPH: Record<Tone, IconName> = {
		info: 'info',
		warn: 'triangle-alert',
		bad: 'circle-alert',
		ok: 'circle-check'
	};

	// Named after the sentence it closes rather than "Close", because a screen
	// reader meeting three of these in a page hears the same word three times
	// and cannot tell which banner it is on.
	const closeName = $derived(
		dismissLabel ?? (title === undefined ? 'Dismiss this message' : `Dismiss: ${title}`)
	);

	let root = $state<HTMLElement | null>(null);

	// Every caller removes the whole banner in response to `onDismiss`, so the
	// button that was just pressed leaves the document with it. The region it
	// sat in is read before that happens, because afterwards this component has
	// no handle on anything.
	async function close() {
		const region = root?.parentElement ?? null;
		onDismiss?.();
		await tick();
		if (afterBannerDismissed({ previous: false, region: region?.isConnected === true }) !== 'region') {
			return;
		}
		if (region !== null) {
			focusRegion(region);
		}
	}

	// Escape closes it too, and only while focus is inside this banner: the
	// handler sits on the banner's own element rather than on the window, so a
	// dialog or a menu open over it keeps the key. The press is stopped here
	// once it has been acted on, for the same reason.
	function keyed(event: KeyboardEvent) {
		if (event.key !== 'Escape') {
			return;
		}
		event.stopPropagation();
		void close();
	}
</script>

<!-- The close control precedes the action in DOM order, and therefore in tab
     order, because the phone layout paints it first. Lifting it there with CSS
     `order` instead would have left the two banners that carry both controls
     reading top-to-bottom in one order and tabbing in the other. `has-act` is
     what lets the sheet tell those two apart from the banners whose only
     control is the close. -->
<!-- The key handler is on the banner rather than on its close control so that
     Escape works from wherever focus is inside the banner, which on a banner
     carrying an action is one of two places. The element is not itself a
     control: everything Escape does here the close button does too, and it is
     the button that carries the name and the focus ring. -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
	class="banner {tone === 'info' ? '' : tone}{action ? ' has-act' : ''}"
	bind:this={root}
	onkeydown={onDismiss ? keyed : undefined}
>
	<span class="ico"><Icon name={GLYPH[tone]} size={17} /></span>
	<div class="say">
		{#if title}<div class="t">{title}</div>{/if}
		<p>{@render children()}</p>
	</div>
	{#if onDismiss}
		<button type="button" class="banner-close" aria-label={closeName} onclick={close}>
			<Icon name="x" size={17} />
		</button>
	{/if}
	{#if action}<span class="act">{@render action()}</span>{/if}
</div>
