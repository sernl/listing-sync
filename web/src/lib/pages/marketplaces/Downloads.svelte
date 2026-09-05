<script lang="ts">
	import Button from '$lib/Button.svelte';
	import StatusPill from '$lib/StatusPill.svelte';
	import type { DownloadsManifest } from './api';
	import { downloadCards } from './downloads';

	let { manifest }: { manifest: DownloadsManifest | null } = $props();

	const cards = $derived(downloadCards(manifest));
</script>

<section id="downloads">
	<div class="mp-sect">
		<h2>Downloads</h2>
		<p>
			The desktop app is what runs the work for TES and TPT on your own machine, and the only
			surface that can run it while you are away.
		</p>
	</div>

	<div class="mp-grid">
		{#each cards as card (card.platform)}
			<article class="mp-card" class:pending={card.offer === undefined}>
				<div class="mp-cap">
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
