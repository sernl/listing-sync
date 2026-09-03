<script lang="ts">
	import { ApiFailure, api, type LabelView } from '$lib/api';
	import type { InventoryRow } from '$lib/inventory';
	import type { LabelColour } from '$lib/generated/vocab';

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

	const suggestions = $derived(
		known.filter(
			(label) => !adding.some((held) => held.toLowerCase() === label.name.toLowerCase())
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
				const merged = [...held.labels.map((label) => label.name)];
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
					: 'That item could not be relabelled.';
			refusal =
				count === 0
					? reason
					: `${count} of ${rows.length} ${count === 1 ? 'item was' : 'items were'} relabelled, then: ${reason}`;
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
			Label {rows.length} {rows.length === 1 ? 'item' : 'items'}
		</h2>
		<p>
			Labels are your own words for filing a catalogue: a term, a bundle, a sale. They are yours
			alone and reach no marketplace.
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
			<p class="foot-note">Labels you already use:</p>
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

		<p class="foot-note">
			These are added to what each item already carries; nothing is taken away. To remove a label
			from one item, open the item.
		</p>

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

	/* The eight of migration 0046's closed set. Tokens rather than fixed
	   triples: the console has a light and a dark theme, and one hex per
	   colour would pick one of them. */
	.c-slate {
		border-color: var(--line);
		color: var(--muted);
	}
	.c-red {
		border-color: color-mix(in srgb, #d1495b 40%, var(--line));
		color: #d1495b;
	}
	.c-amber {
		border-color: color-mix(in srgb, #b3760e 40%, var(--line));
		color: #b3760e;
	}
	.c-green {
		border-color: color-mix(in srgb, #2f8f5b 40%, var(--line));
		color: #2f8f5b;
	}
	.c-teal {
		border-color: color-mix(in srgb, #10808c 40%, var(--line));
		color: #10808c;
	}
	.c-blue {
		border-color: color-mix(in srgb, #2f6fb3 40%, var(--line));
		color: #2f6fb3;
	}
	.c-violet {
		border-color: color-mix(in srgb, #6a4bb3 40%, var(--line));
		color: #6a4bb3;
	}
	.c-pink {
		border-color: color-mix(in srgb, #b3407f 40%, var(--line));
		color: #b3407f;
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
