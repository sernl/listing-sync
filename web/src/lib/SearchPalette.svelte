<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { goto } from '$app/navigation';
	import { allPages, api } from '$lib/api';
	import Icon from '$lib/Icon.svelte';
	import { searchHref } from '$lib/nav';
	import { queryKeys } from '$lib/query';
	import {
		clampHighlight,
		coverToDraw,
		isStale,
		keyAction,
		paletteAnchor,
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

	// Which cover URLs failed to load. A set rather than one string, because
	// this component draws every row rather than one, and reassigned rather
	// than mutated so the rows re-derive.
	let failedCovers = $state<ReadonlySet<string>>(new Set());

	function coverFailed(url: string) {
		if (!failedCovers.has(url)) {
			failedCovers = new Set(failedCovers).add(url);
		}
	}

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

	// Where the phone sheet sits, measured rather than assumed. `visualViewport`
	// reports the band an on-screen keyboard has left visible, which is the one
	// measurement that is right whether or not the browser honours
	// `interactive-widget=resizes-content`; the stylesheet reads the answer
	// through `--pal-top` and `--pal-max` and only below its own phone
	// breakpoint, so the properties are written at every width and read at one.
	// Absent the API -- `undefined` on a browser that never implemented it,
	// which is why the guard below tests for both -- nothing is written and the
	// fallbacks in `shell.css` place the sheet instead.
	$effect(() => {
		const sheet = element;
		const band = window.visualViewport;
		if (!open || sheet === null || band == null) {
			return;
		}
		const place = () => {
			const { top, maxHeight } = paletteAnchor({ height: band.height, offsetTop: band.offsetTop });
			sheet.style.setProperty('--pal-top', `${top}px`);
			sheet.style.setProperty('--pal-max', `${maxHeight}px`);
		};
		place();
		// The visual viewport's own events rather than the window's: the window
		// does not resize when the keyboard opens on a browser that shrinks only
		// the visual viewport, which is the case this exists for.
		band.addEventListener('resize', place);
		band.addEventListener('scroll', place);
		return () => {
			band.removeEventListener('resize', place);
			band.removeEventListener('scroll', place);
			sheet.style.removeProperty('--pal-top');
			sheet.style.removeProperty('--pal-max');
		};
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
				{@const cover = coverToDraw(row.cover, failedCovers)}
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
					{#if cover !== null}
						<!-- Decorative: the title beside it names the resource, and a
						     screen reader reading the cover twice is worse than not
						     reading it. Eagerly loaded on purpose -- a palette shows
						     eight rows at most and they belong on the first frame.
						     A cover that fails to load falls back to the placeholder
						     below rather than leaving a broken-image glyph in the box. -->
						<img
							class="pal-thumb"
							src={cover}
							alt=""
							width="32"
							height="32"
							onerror={() => coverFailed(cover)}
						/>
					{:else}
						<span class="pal-thumb pal-thumb-none" aria-hidden="true">
							<Icon name="image" size={15} />
						</span>
					{/if}
					<span class="pal-text">
						<span class="pal-title">{row.title}</span>
						<span class="pal-meta">{row.meta}</span>
					</span>
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
			<p class="pal-said">Loading your resources…</p>
		{:else if view.kind === 'failed'}
			<!-- The console's own sentence. What the transport said belongs on the
			     board, which can offer a retry; a palette that reported a status
			     code would be asking the seller to act on something they cannot. -->
			<p class="pal-said pal-bad">We couldn't load your resources, so search isn't working.</p>
		{:else if view.kind === 'none'}
			<p class="pal-said">No resource matches “{view.query}”.</p>
		{:else if overflow !== null}
			<a class="pal-more" href={searchHref(query)} onclick={dismiss}>
				Showing {rows.length} of {overflow} matches. See them all in Resources
			</a>
		{:else if view.total !== null}
			<p class="pal-said">
				{view.total === 1 ? '1 match' : `${view.total} matches`} · Press Enter to open the highlighted one
			</p>
		{:else}
			<!-- A source serving one page at a time knows of no total, and the
			     page's own length is not one, so nothing is counted here. -->
			<p class="pal-said">Press Enter to open the highlighted one.</p>
		{/if}

		{#if stale}
			<!-- Said beside the answer rather than in place of it: these rows are
			     searchable and worth showing, and what they cannot account for is
			     anything added since the read that failed. -->
			<p class="pal-said pal-stale">
				These results may be out of date. Anything added since the last load is missing.
			</p>
		{/if}
	</div>
</dialog>
