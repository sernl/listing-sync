<script lang="ts">
	import type { Snippet } from 'svelte';
	import Icon from '$lib/Icon.svelte';
	import type { IconName } from '$lib/icons';

	type Tone = 'info' | 'warn' | 'bad' | 'ok';

	let {
		tone = 'info',
		title,
		children,
		action
	}: {
		tone?: Tone;
		title?: string;
		children: Snippet;
		action?: Snippet;
	} = $props();

	const GLYPH: Record<Tone, IconName> = {
		info: 'info',
		warn: 'triangle-alert',
		bad: 'circle-alert',
		ok: 'circle-check'
	};
</script>

<div class="banner {tone === 'info' ? '' : tone}">
	<span class="ico"><Icon name={GLYPH[tone]} size={17} /></span>
	<div class="say">
		{#if title}<div class="t">{title}</div>{/if}
		<p>{@render children()}</p>
	</div>
	{#if action}<span class="act">{@render action()}</span>{/if}
</div>
