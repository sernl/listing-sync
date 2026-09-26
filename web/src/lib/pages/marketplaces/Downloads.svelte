<script lang="ts">
	import type { Snippet } from 'svelte';
	import Button from '$lib/Button.svelte';
	import Explain from '$lib/Explain.svelte';
	import TileMark from './TileMark.svelte';
	import MarkLink from './MarkLink.svelte';
	import type { DownloadsManifest } from './api';
	import { downloadCards } from './downloads';

	let {
		manifest,
		soon
	}: {
		manifest: DownloadsManifest | null;
		/** More entries for the "Coming soon" row, drawn after the platforms
		 *  with no published build: the browser extensions. */
		soon?: Snippet;
	} = $props();

	const cards = $derived(downloadCards(manifest));
	const offered = $derived(cards.filter((card) => card.offer !== undefined));
	const waiting = $derived(cards.filter((card) => card.offer === undefined));
</script>

<div id="downloads" class="mp-downloads">
	{#if offered.length > 0}
		<div class="mp-dl-grid">
			{#each offered as card (card.platform)}
				<article class="mp-dl">
					<!-- The same mark box the marketplace tiles carry. Never a link:
					     these tiles offer a file, not a page of the platform's owner. -->
					<span class="mp-mark mp-square" aria-hidden="true">
						<TileMark mark={card.mark} />
					</span>
					<span class="mp-dl-name">{card.name}</span>
					{#if card.offer}
						<span class="mp-version">Version {card.offer.version}</span>
						<Button tier="primary" small icon="download" href={card.offer.href}>
							{card.offer.label}
						</Button>
					{/if}
				</article>
			{/each}
		</div>
	{/if}

	<div class="mp-soon">
		<span class="mp-soon-label">Coming soon</span>
		<div class="mp-soon-marks">
			{#each waiting as card (card.platform)}
				<MarkLink mark={card.mark} name={card.name} />
			{/each}
			{@render soon?.()}
		</div>
	</div>

	{#if offered.length > 0}
		<div class="flow-actions">
			<Explain title="Check your download" label="Check the file">
				<p>Each file's SHA-256 fingerprint. Your computer can compute it and compare.</p>
				{#each offered as card (card.platform)}
					{#if card.offer}
						<p>
							<strong>{card.name}</strong>, version {card.offer.version}<br />
							<code class="mp-sha">{card.offer.sha256}</code>
						</p>
					{/if}
				{/each}
			</Explain>
			<Explain title="What each app does" label="">
				{#each offered as card (card.platform)}
					<p><strong>{card.name}.</strong> {card.body}</p>
				{/each}
			</Explain>
		</div>
	{/if}
</div>
