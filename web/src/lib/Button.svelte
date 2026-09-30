<script lang="ts">
	import type { Snippet } from 'svelte';
	import Icon from '$lib/Icon.svelte';
	import type { IconName } from '$lib/icons';

	/** The four tiers, in the order they claim attention.
	 *
	 * `primary` is the one top-priority action on a page; `additive` adds or
	 * confirms; `outline` is neutral; `quiet` must not compete. `danger` is a
	 * modifier on `outline` rather than a tier of its own, because a
	 * destructive action is never the recommended one. */
	type Tier = 'primary' | 'additive' | 'outline' | 'quiet';

	let {
		tier = 'outline',
		icon,
		label,
		href,
		download = false,
		type = 'button',
		danger = false,
		small = false,
		disabled = false,
		/** Why the control cannot run. Required when `disabled`, because a
		 *  disabled control with no stated reason reads as a fault. */
		reason,
		onclick,
		children
	}: {
		tier?: Tier;
		icon?: IconName;
		/** The control's accessible name, set where the button is drawn as its
		 *  glyph alone on a narrow screen. The children stay the word: a
		 *  button that ships no text has nothing to fall back to when the
		 *  glyph fails to load, and nothing for a search to match. */
		label?: string;
		href?: string;
		/** The link is a file to save rather than a page. The browser then
		 *  fetches it itself: without this the console's router takes a
		 *  same-origin `/downloads/…` link as a page of its own and answers
		 *  it with the not-found page. */
		download?: boolean;
		type?: 'button' | 'submit';
		danger?: boolean;
		small?: boolean;
		disabled?: boolean;
		reason?: string;
		onclick?: () => void;
		children: Snippet;
	} = $props();

	const classes = $derived(
		[
			tier === 'primary' || tier === 'additive' ? 'cta' : 'btn',
			tier === 'additive' ? 'add' : '',
			tier === 'quiet' ? 'quiet' : '',
			danger ? 'danger' : '',
			small ? 'small' : '',
			label === undefined ? '' : 'btn-iconic'
		]
			.filter(Boolean)
			.join(' ')
	);
</script>

{#if href !== undefined && !disabled}
	<a
		class={classes}
		{href}
		download={download ? '' : undefined}
		title={reason ?? label}
		aria-label={label}
	>
		{#if icon}<Icon name={icon} size={small ? 14 : 16} />{/if}
		<span class="btn-word">{@render children()}</span>
	</a>
{:else}
	<button
		class={classes}
		{type}
		{disabled}
		title={reason ?? label}
		aria-label={label}
		{onclick}
	>
		{#if icon}<Icon name={icon} size={small ? 14 : 16} />{/if}
		<span class="btn-word">{@render children()}</span>
	</button>
{/if}
