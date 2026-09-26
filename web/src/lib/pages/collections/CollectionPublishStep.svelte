<script lang="ts">
	// Publishing a collection, as a step of the collection's page rather than a
	// dialog over it: the collection drawn as an arrow to the marketplace it is
	// going to, the marketplace and draft-or-live as tiles, a preview of what
	// each member would do, and Publish in the step's sticky footer.
	//
	// Every resource in the collection not already on that marketplace is
	// published, in the collection's order. Changing the marketplace or the
	// intent retires the preview and the idempotency key with it: a preview of
	// another ask is not what Publish would do.

	import {
		ApiFailure,
		collectionsApi,
		type CollectionPublishPlanView,
		type PublishIntent
	} from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import Explain from '$lib/Explain.svelte';
	import { external } from '$lib/external';
	import FlowDiagram from '$lib/FlowDiagram.svelte';
	import FlowStep from '$lib/FlowStep.svelte';
	import type { InventoryId } from '$lib/generated/vocab';
	import { INVENTORY_ORDER } from '$lib/listings-view';
	import MarketplaceMark from '$lib/MarketplaceMark.svelte';
	import { VERDICT_TONE, VERDICT_WORD, countsLine } from '$lib/migration-plan';
	import { AUTHORABLE, SHORT_NAME, platformTitle } from '$lib/platforms';
	import StatusPill from '$lib/StatusPill.svelte';
	import '$lib/flow.css';
	import './collections.css';

	let {
		n,
		collection,
		name,
		count,
		open = $bindable(true),
		onPublished
	}: {
		/** The step's number on the page. */
		n: number;
		collection: string;
		name: string;
		/** How many members the collection holds, for the words before a
		 *  preview has been taken. */
		count: number;
		open?: boolean;
		/** What the submit queued, and the run that carries it where one was
		 *  minted. */
		onPublished: (queued: number, job: string | null) => void;
	} = $props();

	/** Why nothing can be sent to this marketplace. The tile is offered and
	 *  refused rather than hidden: a marketplace left out reads as one this
	 *  console has never heard of. */
	const CANNOT_WRITE = 'You can’t publish to Etsy yet.';
	const EMPTY = 'Add resources to this collection first.';

	let inventory = $state<InventoryId>('Tpt');
	let intent = $state<PublishIntent>('draft');
	let plan = $state<CollectionPublishPlanView | null>(null);
	let previewing = $state(false);
	let planFailure = $state<string | null>(null);
	let sending = $state(false);
	let refusal = $state<string | null>(null);
	/** Minted at the confirm: the server takes the key as the run's identity,
	 *  so it changes when the ask does and stays put across a failed submit's
	 *  retry. */
	let key = $state<string | null>(null);

	$effect(() => {
		void inventory;
		void intent;
		plan = null;
		planFailure = null;
		refusal = null;
		key = null;
	});

	const tileRefusal = $derived(AUTHORABLE[inventory] ? null : CANNOT_WRITE);
	const previewRefusal = $derived(count === 0 ? EMPTY : tileRefusal);

	const confirmRefusal = $derived.by(() => {
		if (previewRefusal !== null) {
			return previewRefusal;
		}
		if (plan === null) {
			return 'Preview first to see what will happen.';
		}
		if (plan.counts.will_create === 0) {
			return `Nothing to publish: every resource here is already on ${SHORT_NAME[inventory]} or can’t be published yet.`;
		}
		return null;
	});

	const INTENT_WORD: Record<PublishIntent, string> = { draft: 'As drafts', live: 'Live' };

	async function preview() {
		previewing = true;
		planFailure = null;
		try {
			plan = await collectionsApi.publishPlan(collection, { inventory, intent });
		} catch (caught) {
			plan = null;
			planFailure =
				caught instanceof ApiFailure ? caught.message : 'We couldn’t load the preview. Try again.';
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
			refusal = caught instanceof ApiFailure ? caught.message : 'We couldn’t start publishing.';
		} finally {
			sending = false;
		}
	}
</script>

<FlowStep
	{n}
	id="publish"
	title="Publish"
	hint="Choose a marketplace, preview, then publish."
	summary="{SHORT_NAME[inventory]} · {INTENT_WORD[intent]}"
	bind:open
>
	{#snippet aside()}
		<Explain title="How a collection is published" label="">
			<p>
				Every resource in {name} that isn’t already on the marketplace is published, in the
				collection’s order. Resources already there are skipped, not published twice.
			</p>
			<p>Previewing doesn’t publish anything.</p>
			<p>A draft waits on the marketplace for you to publish it there. Live goes straight up.</p>
		</Explain>
	{/snippet}

	<FlowDiagram
		from={{ icon: 'layers', label: count === 1 ? '1 resource' : `${count} resources` }}
		to={[{ inventory }]}
		rule={INTENT_WORD[intent]}
		label="Publish {name} to {SHORT_NAME[inventory]} {INTENT_WORD[intent].toLowerCase()}"
	/>

	<div class="flow-choice coll-mk" role="radiogroup" aria-label="Choose a marketplace">
		{#each INVENTORY_ORDER as one (one)}
			{@const why = AUTHORABLE[one] ? null : CANNOT_WRITE}
			<button
				type="button"
				role="radio"
				aria-checked={inventory === one}
				disabled={why !== null || sending}
				title={why ?? undefined}
				onclick={() => (inventory = one)}
			>
				<span class="coll-mk-line">
					<MarketplaceMark inventory={one} size={20} />
					{platformTitle(one)}
				</span>
				{#if why !== null}<span class="sub">{why}</span>{/if}
			</button>
		{/each}
	</div>

	<div class="flow-choice" role="radiogroup" aria-label="Draft or live">
		<button
			type="button"
			role="radio"
			aria-checked={intent === 'draft'}
			disabled={sending}
			onclick={() => (intent = 'draft')}
		>
			As drafts
			<span class="sub">You publish them on the marketplace.</span>
		</button>
		<button
			type="button"
			role="radio"
			aria-checked={intent === 'live'}
			disabled={sending}
			onclick={() => (intent = 'live')}
		>
			Live
			<span class="sub">Buyers see them straight away.</span>
		</button>
	</div>
	{#if intent === 'live' && inventory === 'Tes'}
		<p class="flow-warn">We can’t undo a live publish on Tes.</p>
	{/if}

	{#if planFailure !== null}
		<Banner tone="bad">{planFailure}</Banner>
	{:else if plan !== null}
		{#if plan.rows.length === 0}
			<p class="quiet">This collection is empty, so there’s nothing to publish.</p>
		{:else}
			<p class="coll-tally">{countsLine(plan.counts)}</p>
			<div class="flow-table-wrap">
				<table class="flow-table coll-plan">
					<thead>
						<tr><th>Resource</th><th>Result</th><th>Why</th></tr>
					</thead>
					<tbody>
						{#each plan.rows as row (row.product)}
							<tr>
								<td><span class="res-name">{row.title}</span></td>
								<td class="marks">
									<StatusPill tone={VERDICT_TONE[row.verdict]} label={VERDICT_WORD[row.verdict]} />
								</td>
								<td class="coll-why">
									{#if row.reason !== null}<span>{row.reason}</span>{/if}
									{#if row.remote !== null}
										<a href={row.remote} target="_blank" rel="noreferrer noopener" use:external>
											Open on {SHORT_NAME[plan.inventory]}
										</a>
									{/if}
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
		{/if}
	{/if}

	{#if refusal !== null}
		<Banner tone="bad">{refusal}</Banner>
	{/if}

	{#snippet footer()}
		<Button
			tier={plan === null ? 'primary' : 'outline'}
			icon="eye"
			disabled={previewing || sending || previewRefusal !== null}
			reason={previewRefusal ?? (previewing ? 'Loading the preview.' : undefined)}
			onclick={() => void preview()}
		>
			{previewing ? 'Previewing…' : plan === null ? 'Preview' : 'Preview again'}
		</Button>
		<Button
			tier="primary"
			icon="share-2"
			disabled={confirmRefusal !== null || sending}
			reason={confirmRefusal ?? (sending ? 'Starting to publish.' : undefined)}
			onclick={() => void confirm()}
		>
			{sending ? 'Starting…' : `Publish ${plan?.counts.will_create ?? ''}`.trim()}
		</Button>
		{#if plan === null && previewRefusal === null}
			<span class="coll-foot-note">Preview first.</span>
		{/if}
	{/snippet}
</FlowStep>
