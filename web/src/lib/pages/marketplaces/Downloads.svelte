<script lang="ts">
	import type { Snippet } from 'svelte';
	import Button from '$lib/Button.svelte';
	import Explain from '$lib/Explain.svelte';
	import TileMark from './TileMark.svelte';
	import MarkLink from './MarkLink.svelte';
	import type { DownloadsManifest } from './api';
	import { type Platform, detectPlatform, downloadCards } from './downloads';

	let {
		manifest,
		soon
	}: {
		manifest: DownloadsManifest | null;
		/** More entries for the "Coming soon" row, drawn after the platforms
		 *  with nothing published: the browser extensions. */
		soon?: Snippet;
	} = $props();

	// The console renders in the browser only (`ssr = false`), so the navigator
	// is always there to ask.
	const device: Platform | null = detectPlatform(
		navigator as Navigator & { userAgentData?: { platform?: string } }
	);

	const cards = $derived(downloadCards(manifest, device));
	const offered = $derived(
		cards.filter((card) => card.offer !== undefined || card.store !== undefined)
	);
	const waiting = $derived(
		cards.filter((card) => card.offer === undefined && card.store === undefined)
	);
	const files = $derived(
		offered.flatMap((card) =>
			card.offer === undefined
				? []
				: [card.offer, ...card.alternatives].map((offer) => ({ card, offer }))
		)
	);
</script>

<div id="downloads" class="mp-downloads">
	{#if offered.length > 0}
		<div class="mp-dl-grid">
			{#each offered as card (card.platform)}
				<article class="mp-dl" class:mp-dl-current={card.current}>
					<!-- The same mark box the marketplace tiles carry. Never a link:
					     these tiles offer a file, not a page of the platform's owner. -->
					<span class="mp-mark mp-square" aria-hidden="true">
						<TileMark mark={card.mark} />
					</span>
					<span class="mp-dl-name">{card.name}</span>
					{#if card.current}
						<span class="mp-dl-here">For this device</span>
					{/if}
					{#if card.offer}
						<span class="mp-version">
							Version {card.version}{#if card.offer.size}&nbsp;· {card.offer.size}{/if}
						</span>
						<Button
							tier={card.current ? 'primary' : 'outline'}
							small
							icon="download"
							href={card.offer.href}
							download
						>
							{card.offer.label}
						</Button>
					{/if}
					{#if card.alternatives.length > 0 || card.store}
						<ul class="mp-dl-more">
							{#each card.alternatives as alternative (alternative.file)}
								<li>
									<a href={alternative.href} download>{alternative.label}</a>
									{#if alternative.size}<span class="mp-version">{alternative.size}</span>{/if}
								</li>
							{/each}
							{#if card.store}
								<li>
									<a href={card.store.href} target="_blank" rel="noopener noreferrer">
										{card.store.label}
									</a>
								</li>
							{/if}
						</ul>
					{/if}
				</article>
			{/each}
		</div>
	{/if}

	{#if waiting.length > 0 || soon}
		<div class="mp-soon">
			<span class="mp-soon-label">Coming soon</span>
			<div class="mp-soon-marks">
				{#each waiting as card (card.platform)}
					<MarkLink mark={card.mark} name={card.name} />
				{/each}
				{@render soon?.()}
			</div>
		</div>
	{/if}

	{#if offered.length > 0}
		<div class="flow-actions">
			{#if files.length > 0}
				<Explain title="Check your download" label="Check the file">
					<p>Each file's SHA-256 fingerprint. Your computer can compute it and compare.</p>
					{#each files as { card, offer } (offer.file)}
						<p>
							<strong>{card.name}</strong>, {offer.file}<br />
							<code class="mp-sha">{offer.sha256}</code>
						</p>
					{/each}
				</Explain>
			{/if}
			<Explain title="What each app does" label="">
				{#each offered as card (card.platform)}
					<p><strong>{card.name}.</strong> {card.body}</p>
				{/each}
			</Explain>
		</div>
	{/if}
</div>
