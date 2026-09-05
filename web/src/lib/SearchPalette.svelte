<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { goto } from '$app/navigation';
	import { allPages, api } from '$lib/api';
	import Icon from '$lib/Icon.svelte';
	import { searchHref } from '$lib/nav';
	import { queryKeys } from '$lib/query';
	import {
		clampHighlight,
		isStale,
		keyAction,
		paletteView,
		resultCount,
		type PaletteResult
	} from '$lib/search-palette';

	let { open = $bindable(false) }: { open?: boolean } = $props();

	// The same key and the same walk the Resources board uses, so the palette
	// and the board are one cached answer rather than two that can disagree.
	// `enabled` keeps the walk off every other page until the palette is
	// opened, and `staleTime` keeps a second open from repeating it: without
	// one the default is zero, the rows are stale the instant they arrive, and
	// flipping `enabled` back to true refetches, so every open would re-walk
	// every page of the catalogue. Infinity is what the other nine reads in
	// this console use.
	const catalogue = createQuery(() => ({
		queryKey: queryKeys.catalogue(null),
		queryFn: () =>
			allPages(
				(cursor) => api.products(cursor, null),
				(view) => view.products
			),
		enabled: open,
		staleTime: Infinity
	}));
	const mappings = createQuery(() => ({
		queryKey: queryKeys.mappings,
		queryFn: () => api.mappings().then((view) => view.mappings),
		enabled: open,
		staleTime: Infinity
	}));

	let query = $state('');
	let highlighted = $state(0);
	let element = $state<HTMLDialogElement | null>(null);
	let box = $state<HTMLInputElement | null>(null);
	// Where focus was when the palette opened. Escape returns the seller to the
	// page they were on, and to the control they left, rather than to the top
	// of the document.
	let cameFrom: HTMLElement | null = null;

	const view = $derived(
		paletteView({
			query,
			catalogue: catalogue.data ?? null,
			mappings: mappings.data ?? null,
			failed: catalogue.isError
		})
	);
	const count = $derived(resultCount(view));
	// Read through the clamp rather than trusted: the results are recomputed on
	// every keystroke, so the index held here can point past the list that is
	// actually showing.
	const active = $derived(clampHighlight(highlighted, count));
	const rows = $derived<PaletteResult[]>(view.kind === 'results' ? view.rows : []);
	const activeId = $derived(count > 0 ? `palette-result-${active}` : undefined);
	const stale = $derived(isStale(view));
	const overflow = $derived(
		view.kind === 'results' && view.total !== null && view.total > view.rows.length
			? view.total
			: null
	);

	// `showModal` on an already-modal dialog throws, and this effect re-runs
	// whenever the element binds as well as when `open` moves, so both
	// directions are guarded on the element's own state.
	$effect(() => {
		if (open && element !== null && !element.open) {
			cameFrom = document.activeElement instanceof HTMLElement ? document.activeElement : null;
			element.showModal();
			box?.focus();
		} else if (!open) {
			element?.close();
		}
	});

	// The list scrolls, so arrowing past its foot would otherwise leave the
	// highlight below the fold and Enter would open a resource the seller never
	// saw highlighted -- the same hazard `clampHighlight` guards for a stale
	// index, in the direction it cannot reach.
	$effect(() => {
		if (count === 0) {
			return;
		}
		document
			.getElementById(`palette-result-${active}`)
			?.scrollIntoView({ block: 'nearest' });
	});

	function dismiss() {
		open = false;
		query = '';
		highlighted = 0;
		cameFrom?.focus();
	}

	function openResult(index: number) {
		const row = rows[index];
		if (row === undefined) {
			return;
		}
		// Dismissed before the navigation rather than after: the palette sits in
		// the top layer over every route, and closing it second would leave it
		// standing over the resource it just opened.
		dismiss();
		void goto(row.href);
	}

	function keyed(event: KeyboardEvent) {
		const action = keyAction({
			key: event.key,
			count,
			highlighted: active,
			isComposing: event.isComposing
		});
		switch (action.kind) {
			case 'move':
				event.preventDefault();
				highlighted = action.to;
				return;
			case 'open':
				event.preventDefault();
				openResult(action.index);
				return;
			case 'close':
				event.preventDefault();
				dismiss();
				return;
			case 'ignore':
				return;
		}
	}

	// A press on the backdrop of a modal dialog lands on the dialog element
	// itself, which is the whole of "click outside" here.
	function pressed(event: MouseEvent) {
		if (event.target === element) {
			dismiss();
		}
	}
</script>

<dialog
	class="pal-box"
	bind:this={element}
	aria-label="Search resources"
	onclick={pressed}
	oncancel={(event) => {
		event.preventDefault();
		dismiss();
	}}
>
	<div class="pal-head">
		<Icon name="search" size={17} />
		<label class="sr-only" for="palette-query">Search resources</label>
		<!-- `type="text"` rather than `search`: this input carries the combobox
		     role, and the user agent's own clear affordance on a search field
		     sits inside a control whose expanded state and active option are
		     already being announced. -->
		<input
			id="palette-query"
			type="text"
			role="combobox"
			autocomplete="off"
			spellcheck="false"
			aria-expanded={count > 0}
			aria-controls={count > 0 ? 'palette-results' : undefined}
			aria-activedescendant={activeId}
			aria-autocomplete="list"
			placeholder="Search resources…"
			bind:this={box}
			bind:value={query}
			oninput={() => (highlighted = 0)}
			onkeydown={keyed}
		/>
		<kbd class="pal-key">esc</kbd>
	</div>

	{#if count > 0}
		<div class="pal-list" id="palette-results" role="listbox" aria-label="Matching resources">
			{#each rows as row, index (row.id)}
				<button
					class="pal-hit"
					class:pal-on={index === active}
					id="palette-result-{index}"
					type="button"
					role="option"
					tabindex="-1"
					aria-selected={index === active}
					onmouseenter={() => (highlighted = index)}
					onclick={() => openResult(index)}
				>
					<span class="pal-title">{row.title}</span>
					<span class="pal-meta">{row.meta}</span>
				</button>
			{/each}
		</div>
	{/if}

	<!-- A live region, because every one of the five answers is written here and
	     focus never leaves the input: without it a screen-reader user is told
	     the listbox collapsed and never told whether that was no matches, a
	     failed read, or a read still running. -->
	<div class="pal-foot" role="status">
		{#if view.kind === 'blank'}
			<p class="pal-said">Type to find a resource by its title.</p>
		{:else if view.kind === 'reading'}
			<p class="pal-said">Reading your resources…</p>
		{:else if view.kind === 'failed'}
			<!-- The console's own sentence. What the transport said belongs on the
			     board, which can offer a retry; a palette that reported a status
			     code would be asking the seller to act on something they cannot. -->
			<p class="pal-said pal-bad">Your resources could not be read, so this cannot search them.</p>
		{:else if view.kind === 'none'}
			<p class="pal-said">No resource matches “{view.query}”.</p>
		{:else if overflow !== null}
			<a class="pal-more" href={searchHref(query)} onclick={dismiss}>
				Showing {rows.length} of {overflow} matches — open them all in Resources
			</a>
		{:else if view.total !== null}
			<p class="pal-said">
				{view.total === 1 ? '1 match' : `${view.total} matches`} · Enter opens the highlighted one
			</p>
		{:else}
			<!-- A source serving one page at a time knows of no total, and the
			     page's own length is not one, so nothing is counted here. -->
			<p class="pal-said">Enter opens the highlighted one.</p>
		{/if}

		{#if stale}
			<!-- Said beside the answer rather than in place of it: these rows are
			     searchable and worth showing, and what they cannot account for is
			     anything added since the read that failed. -->
			<p class="pal-said pal-stale">
				This is the copy last read — the newest read failed, so anything added since is missing.
			</p>
		{/if}
	</div>
</dialog>
