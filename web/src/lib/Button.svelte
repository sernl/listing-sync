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
		href,
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
		href?: string;
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
			small ? 'small' : ''
		]
			.filter(Boolean)
			.join(' ')
	);
</script>

{#if href !== undefined && !disabled}
	<a class={classes} {href} title={reason}>
		{#if icon}<Icon name={icon} size={small ? 14 : 16} />{/if}
		{@render children()}
	</a>
{:else}
	<button class={classes} {type} {disabled} title={reason} {onclick}>
		{#if icon}<Icon name={icon} size={small ? 14 : 16} />{/if}
		{@render children()}
	</button>
{/if}
