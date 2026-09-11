<script lang="ts">
	import type { InventoryId, Marketplace } from '$lib/generated/vocab';
	import { inventoryMark, marketplaceMark, type MarkView } from '$lib/marketplace-mark';

	type Of =
		| { inventory: InventoryId; marketplace?: undefined }
		| { marketplace: Marketplace; inventory?: undefined };

	let { size = 20, ...of }: Of & { size?: number } = $props();

	const shown: MarkView = $derived(
		of.inventory === undefined ? marketplaceMark(of.marketplace) : inventoryMark(of.inventory)
	);
</script>

<!-- One element with the role and the name, so a screen reader announces the
     platform once and in words; the image inside says nothing on its own.
     A `title` as well, because a mark is only a name to a reader who already
     knows it. -->
<span
	class="mpmark"
	role="img"
	aria-label={shown.name}
	title={shown.name}
	style="--mpmark-size: {size}px"
>
	{#if shown.src === null}
		<span class="word">{shown.name}</span>
	{:else}
		<img src={shown.src} alt="" width={size} height={size} />
	{/if}
</span>

<style>
	.mpmark {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		vertical-align: middle;
		line-height: 1;
	}

	.mpmark img {
		width: var(--mpmark-size);
		height: var(--mpmark-size);
		object-fit: contain;
		border-radius: 5px;
		flex: none;
	}

	.word {
		font-weight: 600;
	}
</style>
