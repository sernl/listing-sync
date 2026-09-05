<script lang="ts">
	import type { Snippet } from 'svelte';
	import Icon from '$lib/Icon.svelte';
	import Menu from '$lib/Menu.svelte';
	import { closes } from '$lib/menu-dismissal';

	let {
		href,
		title,
		meta,
		cover,
		selectable = false,
		selected = $bindable(false),
		onselect,
		strip,
		menu,
		menuOpen = $bindable(false)
	}: {
		href?: string;
		title: string;
		/** One line, because a row that lists four facts is read as none. */
		meta: string;
		cover?: string | null;
		selectable?: boolean;
		selected?: boolean;
		onselect?: (value: boolean) => void;
		/** The marketplace chips, which the caller renders because only it
		 *  knows which marketplaces this row carries. */
		strip?: Snippet;
		/** The kebab's menu items. The kebab itself belongs to the row rather
		 *  than to the caller, so every row opens its menu the same way.
		 *
		 *  The snippet is handed a `close`, because the caller renders the items
		 *  and is therefore the only one that knows a choice has been made: an
		 *  item handler calls it and the menu goes, rather than standing open
		 *  over the thing it just acted on. Callers that would rather drive the
		 *  menu themselves can `bind:menuOpen` instead; both move the one piece
		 *  of state. */
		menu?: Snippet<[() => void]>;
		/** Whether the kebab's menu is showing. Bindable, so a caller that wants
		 *  to drive it can; most callers use the snippet's `close` instead. */
		menuOpen?: boolean;
	} = $props();

	function chose() {
		if (closes({ kind: 'choice' })) {
			menuOpen = false;
		}
	}

	// Which cover URL failed to load, rather than a bare flag: a row that is
	// handed a different cover must try it, and a flag would keep showing the
	// placeholder for a picture nothing has been asked for yet.
	let failed = $state<string | null>(null);
	const drawable = $derived(cover != null && cover !== failed);
</script>

<div class="row-card">
	{#if selectable}
		<input
			type="checkbox"
			bind:checked={selected}
			onchange={(event) => onselect?.(event.currentTarget.checked)}
			aria-label={`Select ${title}`}
		/>
	{/if}

	<!-- The alt is empty on purpose: the title beside it names the resource, and
	     a second name here would be read out twice. Lazily loaded, because a
	     catalogue draws sixty of these and only the first few are on screen. -->
	<span class="thumb">
		{#if drawable}
			<img
				src={cover}
				alt=""
				loading="lazy"
				decoding="async"
				onerror={() => (failed = cover ?? null)}
			/>
		{:else}
			<Icon name="image" size={24} />
		{/if}
	</span>

	<div class="who">
		{#if href}
			<a class="t" {href}>{title}</a>
		{:else}
			<div class="t">{title}</div>
		{/if}
		<div class="meta">{meta}</div>
		{#if strip}<div class="strip">{@render strip()}</div>{/if}
	</div>

	{#if menu}
		<Menu bind:open={menuOpen} label={`Actions for ${title}`}>
			{#snippet trigger()}
				<button
					class="kebab"
					type="button"
					aria-haspopup="menu"
					aria-expanded={menuOpen}
					aria-label={`Actions for ${title}`}
					onclick={() => (menuOpen = !menuOpen)}
				>
					<Icon name="ellipsis-vertical" size={17} />
				</button>
			{/snippet}
			{@render menu(chose)}
		</Menu>
	{/if}
</div>
