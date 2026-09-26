<script lang="ts">
	// Publishing a whole collection to one marketplace: pick the marketplace,
	// pick draft or live, read what it would do member by member, then confirm.
	//
	// The preview is not a courtesy here. A member already on the marketplace
	// is not published again and a member whose listing is blocked cannot be,
	// so a seller who confirmed blind would be told a figure they could not
	// account for. The plan says which is which before anything is spent, and
	// the submit re-derives it rather than trusting what is on screen — the
	// same contract the migration preview has.

	import {
		ApiFailure,
		collectionsApi,
		type CollectionPublishPlanView,
		type PublishIntent
	} from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { external } from '$lib/external';
	import type { InventoryId } from '$lib/generated/vocab';
	import { INVENTORY_ORDER } from '$lib/listings-view';
	import MarketplaceMark from '$lib/MarketplaceMark.svelte';
	import Note from '$lib/Note.svelte';
	import { VERDICT_TONE, VERDICT_WORD, countsLine } from '$lib/migration-plan';
	import { AUTHORABLE, SHORT_NAME, platformTitle } from '$lib/platforms';
	import StatusPill from '$lib/StatusPill.svelte';
	import './collections.css';

	let {
		open,
		collection,
		name,
		count,
		onClose,
		onPublished
	}: {
		open: boolean;
		collection: string;
		name: string;
		/** How many members the collection holds, for the words before a
		 *  preview has been taken. */
		count: number;
		onClose: () => void;
		/** What the submit queued, and the run that carries it where one was
		 *  minted. */
		onPublished: (queued: number, job: string | null) => void;
	} = $props();

	/** Why nothing can be sent to this marketplace, or null where it can. The
	 *  tile is offered and refused rather than hidden: a marketplace left out
	 *  reads as one this console has never heard of. */
	const CANNOT_WRITE = 'You can’t publish to Etsy yet.';

	let element = $state<HTMLDialogElement | null>(null);
	let inventory = $state<InventoryId>('Tpt');
	let intent = $state<PublishIntent>('draft');
	let plan = $state<CollectionPublishPlanView | null>(null);
	let previewing = $state(false);
	let planFailure = $state<string | null>(null);
	let sending = $state(false);
	let refusal = $state<string | null>(null);
	/** Minted at the confirm rather than at open: the server takes the key as
	 *  the run's identity, so it has to change when the ask does and stay put
	 *  across a failed submit's retry. */
	let key = $state<string | null>(null);

	$effect(() => {
		if (open && element !== null && !element.open) {
			element.showModal();
		} else if (!open) {
			element?.close();
		}
	});

	// A changed marketplace or intent makes the preview on screen a statement
	// about something the seller is no longer asking for, so it goes rather
	// than being left to be confirmed. The key goes with it.
	$effect(() => {
		void inventory;
		void intent;
		plan = null;
		planFailure = null;
		refusal = null;
		key = null;
	});

	const tileRefusal = $derived(AUTHORABLE[inventory] ? null : CANNOT_WRITE);

	const confirmRefusal = $derived.by(() => {
		if (tileRefusal !== null) {
			return tileRefusal;
		}
		if (plan === null) {
			return 'Preview first to see what will happen.';
		}
		if (plan.counts.will_create === 0) {
			return `Nothing to publish: every resource here is already on ${SHORT_NAME[inventory]} or can’t be published yet.`;
		}
		return null;
	});

	async function preview() {
		previewing = true;
		planFailure = null;
		try {
			plan = await collectionsApi.publishPlan(collection, { inventory, intent });
		} catch (caught) {
			plan = null;
			planFailure =
				caught instanceof ApiFailure
					? caught.message
					: 'We couldn’t load the preview. Try again.';
		} finally {
			previewing = false;
		}
	}

	async function confirm() {
		key ??= crypto.randomUUID();
		sending = true;
		refusal = null;
		try {
			const ack = await collectionsApi.publish(collection, { inventory, intent }, key);
			plan = null;
			onPublished(ack.queued, ack.job);
		} catch (caught) {
			refusal =
				caught instanceof ApiFailure
					? caught.message
					: 'We couldn’t start publishing.';
		} finally {
			sending = false;
		}
	}
</script>

<dialog
	class="coll-dialog collections-page"
	bind:this={element}
	aria-labelledby="coll-publish-title"
	onclose={onClose}
>
	<div class="dialog-body">
		<h2 id="coll-publish-title">Publish {name} to a marketplace</h2>
		<p>
			Publishes every resource not already on the marketplace, in the collection's order. {count === 1 ? 'It holds 1 resource' : `It holds ${count} resources`}.
		</p>

		<div class="mk-tiles" role="radiogroup" aria-label="Choose a marketplace">
			{#each INVENTORY_ORDER as one (one)}
				{@const why = AUTHORABLE[one] ? null : CANNOT_WRITE}
				<button
					type="button"
					class="mk-tile"
					class:on={inventory === one}
					role="radio"
					aria-checked={inventory === one}
					disabled={why !== null || sending}
					title={why ?? undefined}
					onclick={() => (inventory = one)}
				>
					<MarketplaceMark inventory={one} size={20} />
					<span>
						{platformTitle(one)}
						{#if why !== null}<span class="mk-tile-why">{why}</span>{/if}
					</span>
				</button>
			{/each}
		</div>

		<div class="inline-choices" role="radiogroup" aria-label="Draft or live">
			<label>
				<input
					type="radio"
					name="coll-intent"
					value="draft"
					checked={intent === 'draft'}
					disabled={sending}
					onchange={() => (intent = 'draft')}
				/>
				Draft
			</label>
			<label>
				<input
					type="radio"
					name="coll-intent"
					value="live"
					checked={intent === 'live'}
					disabled={sending}
					onchange={() => (intent = 'live')}
				/>
				Live
			</label>
		</div>
		<Note icon="triangle-alert">We can’t undo a live publish on Tes.</Note>

		<div class="coll-verb-foot">
			<Button
				disabled={previewing || tileRefusal !== null}
				reason={tileRefusal ?? (previewing ? 'Loading the preview.' : undefined)}
				onclick={() => void preview()}
			>
				{previewing ? 'Previewing…' : 'Preview'}
			</Button>
		</div>

		{#if planFailure !== null}
			<Banner tone="bad">{planFailure}</Banner>
		{:else if plan === null}
			<Note>Previewing doesn’t publish anything.</Note>
		{:else if plan.rows.length === 0}
			<Note>This collection is empty, so there’s nothing to publish.</Note>
		{:else}
			<div class="plan-rows">
				{#each plan.rows as row (row.product)}
					<div class="plan-row">
						<span class="plan-title">{row.title}</span>
						<StatusPill tone={VERDICT_TONE[row.verdict]} label={VERDICT_WORD[row.verdict]} />
						{#if row.reason !== null || row.remote !== null}
							<span class="plan-why">
								{#if row.remote !== null}
									<a
										class="block"
										href={row.remote}
										target="_blank"
										rel="noreferrer noopener"
										use:external
									>
										Open the listing on {SHORT_NAME[plan.inventory]}
									</a>
								{/if}
								{#if row.reason !== null}<span class="block">{row.reason}</span>{/if}
							</span>
						{/if}
					</div>
				{/each}
			</div>
			<Note>{countsLine(plan.counts)}</Note>
		{/if}

		{#if refusal !== null}
			<Banner tone="bad">{refusal}</Banner>
		{/if}

		<div class="actions">
			<Button
				onclick={onClose}
				disabled={sending}
				reason={sending ? 'Starting to publish.' : undefined}
			>
				Cancel
			</Button>
			<Button
				tier="primary"
				disabled={confirmRefusal !== null || sending}
				reason={confirmRefusal ?? (sending ? 'Starting to publish.' : undefined)}
				onclick={() => void confirm()}
			>
				{sending
					? 'Starting…'
					: `Publish ${plan?.counts.will_create ?? 0}`}
			</Button>
		</div>
	</div>
</dialog>
