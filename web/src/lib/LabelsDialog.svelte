<script lang="ts">
	import { ApiFailure, api, type LabelView } from '$lib/api';
	import type { InventoryRow } from '$lib/inventory';
	import type { LabelColour } from '$lib/generated/vocab';
	import Note from '$lib/Note.svelte';

	/** The class each stored colour renders as.
	 *
	 *  A total map over the generated union rather than the colour name used
	 *  as a class directly, so a colour added to the closed set in Rust stops
	 *  this file type-checking instead of rendering as an unstyled chip. That
	 *  is the guard migration 0046 claims for the closed set, made real. */
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

	function swatch(colour: string): string {
		return SWATCH[colour as LabelColour] ?? SWATCH.slate;
	}

	let {
		open,
		rows,
		known,
		onClose,
		onLabelled,
		onPartial
	}: {
		open: boolean;
		/** The selected rows, in the order the table shows them. */
		rows: InventoryRow[];
		/** Every label the organisation already uses, offered so a seller adds
		 *  the label they have rather than a second spelling of it. */
		known: LabelView[];
		onClose: () => void;
		/** Every selected item was relabelled: the dialog closes and says so. */
		onLabelled: (count: number) => void;
		/** Some items were relabelled and then one failed. The board must
		 *  refresh, because those writes happened, but the dialog stays open
		 *  and nothing claims success. */
		onPartial: (count: number) => void;
	} = $props();

	let element = $state<HTMLDialogElement | null>(null);
	let typed = $state('');
	let adding = $state<string[]>([]);
	let sending = $state(false);
	let refusal = $state<string | null>(null);

	$effect(() => {
		if (open && element !== null && !element.open) {
			element.showModal();
		} else if (!open) {
			element?.close();
		}
	});

	// The seller's own labels alone, and none they have already typed. An
	// import's mark is not a label they can put on anything: the route refuses
	// a set that names one, so offering it here would offer a refusal.
	const suggestions = $derived(
		known.filter(
			(label) =>
				!label.system &&
				!adding.some((held) => held.toLowerCase() === label.name.toLowerCase())
		)
	);

	function add(name: string) {
		const label = name.trim();
		if (label.length === 0) {
			return;
		}
		if (!adding.some((held) => held.toLowerCase() === label.toLowerCase())) {
			adding = [...adding, label];
		}
		typed = '';
	}

	function drop(name: string) {
		adding = adding.filter((held) => held !== name);
	}

	/** Adds rather than replaces: a bulk relabel of twelve items where each
	 *  carries its own labels must not flatten them all to one set, which is
	 *  what sending the typed set alone would do. The per-item screen is where
	 *  a set is replaced. */
	async function apply() {
		if (adding.length === 0) {
			return;
		}
		sending = true;
		refusal = null;
		let count = 0;
		try {
			for (const row of rows) {
				const held = await api.productLabels(row.product.id);
				// The seller's own labels alone. A system label is an import's
				// mark: the server preserves the membership through this
				// replace and refuses a set that names one, so sending it back
				// would refuse a relabel that changes nothing about it.
				const merged = held.labels
					.filter((label) => !label.system)
					.map((label) => label.name);
				for (const name of adding) {
					if (!merged.some((one) => one.toLowerCase() === name.toLowerCase())) {
						merged.push(name);
					}
				}
				await api.setProductLabels(row.product.id, merged);
				count += 1;
			}
			adding = [];
			onLabelled(count);
		} catch (failure) {
			// The loop writes item by item, so a failure part-way leaves earlier
			// items relabelled. Saying how many landed is the difference between
			// a seller retrying the whole selection and retrying the rest, and
			// closing on a success toast here would report a write that did not
			// finish.
			const reason =
				failure instanceof ApiFailure
					? failure.message
					: 'That resource could not be labelled.';
			refusal =
				count === 0
					? reason
					: `Labelled ${count} of ${rows.length}, then it stopped: ${reason}`;
			if (count > 0) {
				onPartial(count);
			}
		} finally {
			sending = false;
		}
	}
</script>

<dialog bind:this={element} aria-labelledby="labels-title" onclose={onClose}>
	<div class="dialog-body">
		<h2 id="labels-title">
			Label {rows.length} {rows.length === 1 ? 'resource' : 'resources'}
		</h2>
		<p>
			Use labels to sort your resources, like a term, a bundle or a sale. Only you see them.
		</p>

		<label class="field">
			<span class="label">Add a label</span>
			<input
				type="text"
				maxlength="60"
				placeholder="Autumn term"
				disabled={sending}
				bind:value={typed}
				onkeydown={(event) => {
					if (event.key === 'Enter') {
						event.preventDefault();
						add(typed);
					}
				}}
			/>
		</label>

		{#if adding.length > 0}
			<div class="chips">
				{#each adding as name (name)}
					<button class="chip" type="button" disabled={sending} onclick={() => drop(name)}>
						{name} <span aria-hidden="true">×</span>
						<span class="sr-only">Remove {name}</span>
					</button>
				{/each}
			</div>
		{/if}

		{#if suggestions.length > 0}
			<Note>Labels you already use:</Note>
			<div class="chips">
				{#each suggestions as label (label.name)}
					<button
						class="chip {swatch(label.colour)}"
						type="button"
						disabled={sending}
						onclick={() => add(label.name)}
					>
						{label.name}
					</button>
				{/each}
			</div>
		{/if}

		<Note>To remove a label, open the resource.</Note>

		{#if refusal !== null}
			<p class="refusal">{refusal}</p>
		{/if}

		<div class="actions">
			<button class="btn" type="button" onclick={onClose} disabled={sending}>Cancel</button>
			<button class="cta" type="button" onclick={apply} disabled={sending || adding.length === 0}>
				{sending ? 'Applying…' : `Apply to ${rows.length}`}
			</button>
		</div>
	</div>
</dialog>

<style>
	.chips {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
		padding: 4px 0;
	}

	.chip {
		font-size: 12px;
		font-weight: 600;
		border-radius: 999px;
		padding: 3px 10px;
		border: 1px solid var(--line);
		background: var(--ground);
		color: var(--ink);
		cursor: pointer;
	}

	/* The eight of migration 0046's closed set, drawn from the `--label-*`
	   tokens `labels.css` also draws, so the two files that repeat this scale
	   read one definition. */
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
