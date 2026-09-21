<script lang="ts">
	import Button from '$lib/Button.svelte';
	import StatusPill from '$lib/StatusPill.svelte';
	import TileMark from './TileMark.svelte';
	import type { DownloadsManifest } from './api';
	import { downloadCards } from './downloads';

	let { manifest }: { manifest: DownloadsManifest | null } = $props();

	const cards = $derived(downloadCards(manifest));
</script>

<section id="downloads">
	<div class="mp-sect">
		<h2>Downloads</h2>
		<p>
			The desktop app does the TES and TPT work on your own computer.
			<a class="mp-guide" href="/guides/connecting">How connecting works</a>
		</p>
	</div>

	<div class="mp-grid">
		{#each cards as card (card.platform)}
			<article class="mp-card" class:pending={card.offer === undefined}>
				<div class="mp-cap">
					<!-- The same mark box the marketplace tiles carry, so a row of download
					     cards reads as the same kind of card rather than as a caption with
					     something missing from it. Never a link: these three tiles offer a
					     file, not a page belonging to the platform's owner. -->
					<span class="mp-mark mp-square">
						<TileMark mark={card.mark} />
					</span>
					<span class="mp-who"><span class="t">{card.name}</span></span>
					{#if card.offer}
						<StatusPill tone="ok" label="Available" />
					{:else}
						<StatusPill tone="soon" label="Coming soon" />
					{/if}
				</div>
				<p class="mp-body">{card.body}</p>
				{#if card.offer}
					<div class="mp-foot">
						<Button tier="primary" icon="download" href={card.offer.href}>
							{card.offer.label}
						</Button>
						<p class="mp-version">
							Version {card.offer.version} · SHA-256 {card.offer.sha256}
						</p>
					</div>
				{/if}
			</article>
		{/each}
	</div>
</section>
