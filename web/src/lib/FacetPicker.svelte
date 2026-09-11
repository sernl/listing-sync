<script lang="ts">
	import { tick } from 'svelte';
	import type { FacetView } from '$lib/api';
	import Button from '$lib/Button.svelte';
	import { closes, doneLabel, focusLeft, pressedInside, returnsFocus } from '$lib/picker-dismissal';
	import { atCap, counterOf, searchFacets, togglePick } from '$lib/tpt-form';

	let {
		label,
		required = false,
		placeholder,
		hint,
		facets,
		chosen,
		cap,
		onChange
	}: {
		label: string;
		required?: boolean;
		placeholder: string;
		hint?: string;
		facets: readonly FacetView[];
		chosen: readonly string[];
		cap: number | null;
		onChange: (values: string[]) => void;
	} = $props();

	const base = $props.id();
	let query = $state('');
	let open = $state(false);
	let anchor = $state<HTMLElement | null>(null);
	let search = $state<HTMLInputElement | null>(null);

	const counter = $derived(counterOf(chosen.length, cap));
	const full = $derived(atCap(chosen.length, cap));
	const matches = $derived(searchFacets(facets, query).slice(0, 60));
	const byslug = $derived(new Map(facets.map((facet) => [facet.slug, facet])));

	function drop(slug: string) {
		onChange(togglePick(chosen, slug, false, cap));
	}

	// Focus goes back to the search box the seller opened the picker from
	// rather than through `$lib/focus-return`: that module answers where focus
	// goes when the control holding it has left the document, and this one is
	// still in it. `held` is whether focus was inside the picker, and a close
	// on focus leaving passes false: pulling focus back from the field the
	// seller tabbed to would trap them here.
	async function close(held = anchor !== null && anchor.contains(document.activeElement)) {
		const coarse = window.matchMedia('(pointer: coarse)').matches;
		open = false;
		await tick();
		if (returnsFocus(held, coarse)) {
			search?.focus();
		}
	}

	function blurred(event: FocusEvent) {
		if (open && focusLeft(event.relatedTarget, anchor) && closes({ kind: 'focusLeft' })) {
			void close(false);
		}
	}

	function pressed(event: PointerEvent) {
		const inside = pressedInside(event.target, anchor);
		if (open && closes({ kind: 'press', inside })) {
			void close();
		}
	}

	function dimmed() {
		if (closes({ kind: 'scrim' })) {
			void close();
		}
	}

	// Escape is stopped as well as answered: its default in a search box
	// holding text is to clear it, and the `input` that clearing dispatches
	// would reopen the picker in the frame it was closed in. Enter on an
	// option is answered here too, because the browser's own answer to it is
	// the form's implicit submission rather than a tick.
	function keyed(event: KeyboardEvent) {
		if (event.key === 'Enter' && isTick(event.target)) {
			event.preventDefault();
			event.target.click();
			return;
		}
		if (event.key !== 'Escape' || !open || !closes({ kind: 'escape' })) {
			return;
		}
		event.preventDefault();
		event.stopPropagation();
		void close();
	}

	function isTick(target: EventTarget | null): target is HTMLInputElement {
		return target instanceof HTMLInputElement && target.type === 'checkbox';
	}

	function done() {
		if (closes({ kind: 'done' })) {
			void close();
		}
	}

	function ticked(slug: string, on: boolean) {
		onChange(togglePick(chosen, slug, on, cap));
		if (closes({ kind: 'pick' })) {
			void close();
		}
	}

	// The list opens on a press, on typing, and on the two keys that mean show
	// me the list, and never on focus: closing puts focus back in this box, and
	// a focus opener would reopen the picker in the frame it was dismissed in.
	// Enter is stopped as well as answered, because this control sits inside
	// the resource form and would otherwise submit it.
	function opener(event: KeyboardEvent) {
		if (event.key !== 'ArrowDown' && event.key !== 'Enter') {
			return;
		}
		event.preventDefault();
		open = true;
	}

	// Below the sheet breakpoint the open picker's box goes to the top of the
	// screen, the one place a sheet rising to 72vh from the bottom cannot
	// cover it, and the filter is typed into that box while the sheet is up.
	$effect(() => {
		if (open && window.matchMedia('(max-width: 620px)').matches) {
			search?.scrollIntoView({ block: 'start' });
		}
	});
</script>

<svelte:window onpointerdown={pressed} />

<!-- TPT renders these as closed react-select comboboxes whose cap lives in the
     placeholder and is discovered by being refused. The control below keeps
     the shape and adds the two things that cost nothing: a live counter
     against the measured cap, and a search over a list that runs to 133
     members. Where no cap is measured the counter reads a plain count rather
     than inventing a ceiling. -->
<!-- Escape is answered on this element rather than on the window, so a dialog
     open over the form keeps the key. The element is not itself a control:
     everything Escape does here the Done button does too, and it is the button
     that carries the name and the focus ring. -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="field fp" bind:this={anchor} onkeydown={keyed} onfocusout={blurred}>
	<span id="{base}-label">
		{label}{#if required}<span class="req">Required</span>{/if}
		<span class="fp-count" class:over={counter.over}>{counter.text}</span>
	</span>
	{#if hint}<span class="hint">{hint}</span>{/if}

	{#if chosen.length > 0}
		<div class="fp-picked" role="list">
			{#each chosen as slug (slug)}
				<span class="fp-chip" role="listitem">
					{byslug.get(slug)?.label ?? slug}
					<button type="button" aria-label="Remove {byslug.get(slug)?.label ?? slug}" onclick={() => drop(slug)}>
						×
					</button>
				</span>
			{/each}
		</div>
	{/if}

	<input
		id="{base}-search"
		type="search"
		{placeholder}
		aria-labelledby="{base}-label"
		class:lifted={open}
		bind:this={search}
		value={query}
		oninput={(event) => {
			query = event.currentTarget.value;
			open = true;
		}}
		onpointerdown={() => (open = true)}
		onkeydown={opener}
	/>

	{#if open}
		<!-- The scrim closes on its click rather than on the press, so the
		     sheet is still under the finger when it lifts; `pressedInside`
		     says what a close on the press lets the tap fall through to. -->
		<div class="fp-scrim" aria-hidden="true" onclick={dimmed}></div>
		<div class="fp-sheet" role="dialog" aria-label={label}>
			<div class="fp-list" role="group" aria-labelledby="{base}-label">
				{#if matches.length === 0}
					<p class="fp-note">Nothing matches “{query}”.</p>
				{:else}
					{#each matches as facet (facet.slug)}
						{@const on = chosen.includes(facet.slug)}
						<label class="fp-tick" class:off={full && !on}>
							<input
								type="checkbox"
								checked={on}
								disabled={full && !on}
								onchange={(event) => ticked(facet.slug, event.currentTarget.checked)}
							/>
							<span>{facet.label}</span>
						</label>
					{/each}
				{/if}
			</div>
			<!-- Polite and on the row rather than the button: Done's words change
			     under a tick while focus is on the tick, and a button's own name
			     changing is not announced. -->
			<div class="fp-acts" aria-live="polite">
				<Button tier="primary" onclick={done}>{doneLabel(chosen.length)}</Button>
				{#if full}
					<span class="hint">That is the limit TPT's own form states.</span>
				{/if}
			</div>
		</div>
	{/if}
</div>

<style>
	/* This picker's own rules, in the component: the wrapper, the label and the
	   hint are `Field`'s own classes so the control reads as one of the
	   catalogue's, and what is here is the search, the chosen chips and the
	   list, which `Field` has no notion of. Tokens only. */

	/* The search control's own shape. It was `app.css`'s `.picker
	   input[type=search]` before this conversion, and dropping that class
	   dropped the border with it. */
	.fp input[type='search'] {
		margin-top: 6px;
		border: 1px solid var(--line);
		background: var(--surface);
		border-radius: var(--r-field);
		padding: 9px 11px;
		font: inherit;
		font-weight: 400;
		color: var(--text);
		min-width: 0;
	}

	.fp input[type='search']:focus-visible {
		border-color: var(--accent);
	}

	.fp-picked {
		margin: 6px 0 2px;
	}

	.fp-count {
		margin-left: 8px;
		font-size: 11.5px;
		font-weight: 400;
		color: var(--muted);
		font-variant-numeric: tabular-nums;
	}

	.fp-count.over {
		color: var(--bad-ink);
		font-weight: 600;
	}

	.fp-picked {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
	}

	.fp-chip {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		border: 1px solid var(--line);
		background: var(--ground);
		border-radius: var(--r-pill);
		padding: 3px 6px 3px 11px;
		font-size: 12.5px;
	}

	.fp-chip button {
		border: 0;
		background: none;
		color: var(--muted);
		font-size: 14px;
		line-height: 1;
		padding: 3px 5px;
		border-radius: 50%;
		cursor: pointer;
	}

	.fp-chip button:hover {
		background: var(--hover);
		color: var(--text);
	}

	/* One box for the whole picker, so DOM order and drawn shape agree:
	   options, hairline, Done. */
	.fp-sheet {
		margin-top: 6px;
		border: 1px solid var(--line);
		border-radius: var(--r-panel);
		background: var(--surface);
		overflow: hidden;
	}

	/* The list runs to 133 members, so it scrolls inside its own box rather
	   than pushing the rest of the form down the page. */
	.fp-list {
		display: flex;
		flex-direction: column;
		gap: 2px;
		max-height: 260px;
		overflow-y: auto;
		padding: 6px;
	}

	.fp-tick {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 6px;
		border-radius: 6px;
		font-size: 13px;
		min-height: 32px;
		cursor: pointer;
	}

	.fp-tick:hover {
		background: var(--hover);
	}

	/* At the cap the rest are refused rather than hidden, so the seller can see
	   what they would have to give up to choose another. */
	.fp-tick.off {
		color: var(--soon);
		cursor: not-allowed;
	}

	.fp-note {
		margin: 0;
		padding: 6px;
		font-size: 12.5px;
		color: var(--muted);
	}

	.fp-acts {
		display: flex;
		align-items: center;
		gap: 12px;
		flex-wrap: wrap;
		border-top: 1px solid var(--line);
		padding: 10px;
	}

	.fp-scrim {
		display: none;
	}

	/* Below 620px the picker is a sheet at the bottom edge over a dimmed page,
	   which is where this console already puts a dialog. 31 clears the phone
	   tab bar at 20 so the Done button is never under it, and stays under the
	   toast stack at 40 so a toast still shows; both are literals like the four
	   z-indexes those sheets already use, since there is no z-index token. */
	@media (max-width: 620px) {
		.fp-scrim {
			display: block;
			position: fixed;
			inset: 0;
			z-index: 30;
			background: var(--scrim);
		}

		.fp-sheet {
			position: fixed;
			inset: auto 0 0;
			z-index: 31;
			margin-top: 0;
			max-height: 72vh;
			border-radius: 14px 14px 0 0;
			border-bottom: 0;
			padding-bottom: env(safe-area-inset-bottom);
		}

		.fp-list {
			max-height: 52vh;
		}

		/* The open picker's own search box stays reachable while its sheet is
		   up: the scrim dims the page around that control rather than it. A
		   press on it is still inside the picker, so the sheet stays open and
		   the keyboard goes on filtering the list. Only the open one is
		   lifted, or all three of the form's pickers would light up at once
		   over a page that is meant to be dimmed. The scroll margin is what
		   the opening scroll leaves above the box: the 18px label line, the
		   field's 5px gap, the box's own 6px margin and the page's 12px
		   gutter, summed here because there is no spacing token. */
		.fp input[type='search'].lifted {
			position: relative;
			z-index: 31;
			scroll-margin-top: 41px;
		}

		/* Stretch rather than a width on the button: Svelte scopes these rules
		   to this component's own markup and `.cta` belongs to
		   `Button.svelte`. */
		.fp-acts {
			flex-direction: column;
			align-items: stretch;
		}
	}
</style>
