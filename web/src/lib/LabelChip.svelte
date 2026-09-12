<script lang="ts">
	// One label, wherever a label is shown: the board's filter, an item's own
	// page, the bulk dialog.
	//
	// Two kinds in one component, because they are one thing to a seller and
	// differ in exactly two ways. A system label is an import's own mark — it
	// carries the marketplace's mark so the seller can see at a glance where
	// the resource came from, and it offers no remove control, because there
	// is nothing here that could remove it: the server preserves the
	// membership through every relabel and refuses a set that names one.
	//
	// The colour comes off the wire either way. A system label's is fixed per
	// marketplace on the server rather than derived from its name, so drawing
	// from a table here would be a second answer that could drift from the
	// labels page's.

	import MarketplaceMark from '$lib/MarketplaceMark.svelte';
	import type { LabelColour } from '$lib/generated/vocab';
	import { marketplaceOfLabel } from '$lib/system-labels';

	let {
		name,
		colour,
		system = false,
		onremove
	}: {
		name: string;
		colour: string;
		system?: boolean;
		/** Offered only where the caller can actually remove it. A system
		 *  label never gets one, whatever the caller passes. */
		onremove?: (name: string) => void;
	} = $props();

	/** The class each stored colour renders as.
	 *
	 *  A total map over the generated union rather than the colour name used
	 *  as a class directly, so a colour added to the closed set in Rust stops
	 *  this file type-checking instead of rendering as an unstyled chip. */
	const SWATCH: Record<LabelColour, string> = {
		slate: 'c-slate',
		red: 'c-red',
		amber: 'c-amber',
		green: 'c-green',
		teal: 'c-teal',
		blue: 'c-blue',
		violet: 'c-violet',
		pink: 'c-pink'
	};

	const swatch = $derived(SWATCH[colour as LabelColour] ?? SWATCH.slate);
	const marketplace = $derived(system ? marketplaceOfLabel(name) : null);
	const removable = $derived(!system && onremove !== undefined);
</script>

{#if removable}
	<button class="chip {swatch}" type="button" onclick={() => onremove?.(name)}>
		{name}
		<span aria-hidden="true">×</span>
		<span class="sr-only">Remove {name}</span>
	</button>
{:else}
	<span class="chip {swatch}" class:sys={system}>
		{#if marketplace !== null}
			<MarketplaceMark {marketplace} size={13} />
		{/if}
		{name}
	</span>
{/if}

<style>
	.chip {
		display: inline-flex;
		align-items: center;
		gap: 5px;
		font-size: 12px;
		font-weight: 600;
		border-radius: 999px;
		padding: 3px 10px;
		border: 1px solid var(--line);
		background: var(--ground);
		color: var(--ink);
	}

	button.chip {
		cursor: pointer;
	}

	/* An import's own mark reads as a fact about the resource rather than as
	   a control, so it takes a filled ground and never a pointer. */
	.sys {
		background: var(--surface);
	}

	/* The eight of migration 0046's closed set, drawn from the `--label-*`
	   tokens `labels.css` also draws. */
	.c-slate {
		border-color: var(--line);
		color: var(--muted);
	}
	.c-red {
		border-color: color-mix(in srgb, var(--label-red) 40%, var(--line));
		color: var(--label-red);
	}
	.c-amber {
		border-color: color-mix(in srgb, var(--label-amber) 40%, var(--line));
		color: var(--label-amber);
	}
	.c-green {
		border-color: color-mix(in srgb, var(--label-green) 40%, var(--line));
		color: var(--label-green);
	}
	.c-teal {
		border-color: color-mix(in srgb, var(--label-teal) 40%, var(--line));
		color: var(--label-teal);
	}
	.c-blue {
		border-color: color-mix(in srgb, var(--label-blue) 40%, var(--line));
		color: var(--label-blue);
	}
	.c-violet {
		border-color: color-mix(in srgb, var(--label-violet) 40%, var(--line));
		color: var(--label-violet);
	}
	.c-pink {
		border-color: color-mix(in srgb, var(--label-pink) 40%, var(--line));
		color: var(--label-pink);
	}

	.sr-only {
		position: absolute;
		width: 1px;
		height: 1px;
		overflow: hidden;
		clip-path: inset(50%);
		white-space: nowrap;
	}
</style>
