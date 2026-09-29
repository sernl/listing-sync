<script lang="ts">
	// Export as three steps: what goes in the spreadsheet (every resource, or
	// one collection), the format, and the download in the last step's sticky
	// footer. What the file carries and the reassurance about files and logins
	// sit behind Explain.

	import { createQuery } from '@tanstack/svelte-query';
	import { api, collectionsApi } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { entitlementRead, limitOf } from '$lib/entitlement-read';
	import Explain from '$lib/Explain.svelte';
	import Field from '$lib/Field.svelte';
	import FlowStep from '$lib/FlowStep.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Stepper, { type StepMark } from '$lib/Stepper.svelte';
	import { COLLECTIONS_KEY } from '$lib/pages/collections/collections';
	import { EMPTINESS_KEY, fetchCatalogueCsv, type ExportSelection } from '$lib/pages/export/api';
	import {
		FILENAME_PATTERN,
		HANDOFF_ADVICE,
		runExport,
		saveDocument,
		type ExportState
	} from '$lib/pages/export/download';
	import '$lib/pages/export/export.css';
	import '$lib/flow.css';

	// One page, not the whole catalogue walk: this asks only whether anything
	// exists, and the first page answers that exactly.
	const catalogue = createQuery(() => ({
		queryKey: EMPTINESS_KEY,
		queryFn: () => api.products()
	}));

	// Collections are a plan feature; a plan without them offers the tile
	// refused, with the plan's own sentence, rather than hiding it.
	const plan = createQuery(() => entitlementRead);
	const collectionsCapped = $derived(limitOf(plan.data, 'collections'));
	const collections = createQuery(() => ({
		queryKey: COLLECTIONS_KEY,
		queryFn: () => collectionsApi.list().then((view) => view.collections),
		enabled: collectionsCapped === null
	}));

	// Fail open. A catalogue that could not be counted is not one known to be
	// empty, and withholding this page's only action over a question the
	// seller never asked would be the worse answer of the two.
	const empty = $derived(catalogue.isSuccess && catalogue.data.products.length === 0);
	const NOTHING_TO_EXPORT = 'There is nothing to export yet.';

	let what = $state<'all' | 'collection'>('all');
	let collection = $state('');

	const heldCollections = $derived(collections.data ?? []);
	const collectionRefusal = $derived.by(() => {
		if (collectionsCapped !== null) {
			return collectionsCapped;
		}
		if (collections.isError) {
			return 'Your collections could not be loaded.';
		}
		if (collections.isSuccess && heldCollections.length === 0) {
			return 'You have no collections yet.';
		}
		return null;
	});
	const chosenCollection = $derived(heldCollections.find((one) => one.id === collection) ?? null);

	const selection = $derived<ExportSelection>(
		what === 'collection' && collection !== '' ? { collection } : null
	);
	const whatLine = $derived(
		what === 'all' ? 'Every resource' : (chosenCollection?.name ?? 'Choose a collection')
	);

	let outcome = $state<ExportState>({ kind: 'idle' });
	const working = $derived(outcome.kind === 'working');

	const blocked = $derived.by(() => {
		if (empty) {
			return NOTHING_TO_EXPORT;
		}
		if (what === 'collection' && collection === '') {
			return 'Choose a collection first.';
		}
		return null;
	});

	async function download() {
		// The button stays focusable while the export runs, so a second press
		// is refused here rather than by removing the control the seller is
		// standing on.
		if (working || blocked !== null) {
			return;
		}
		const asked = selection;
		outcome = { kind: 'working' };
		outcome = await runExport(() => fetchCatalogueCsv(fetch, asked), saveDocument);
	}

	let whatOpen = $state(true);
	let formatOpen = $state(true);
	let downloadOpen = $state(true);

	const steps = $derived<StepMark[]>([
		{ id: 'what', label: 'What', done: what === 'all' || collection !== '' },
		{ id: 'format', label: 'Format', done: true },
		{ id: 'download', label: 'Download', done: outcome.kind === 'handed' }
	]);
</script>

{#snippet signIn()}
	<Button href="/login">Sign in</Button>
{/snippet}

{#snippet downloadButton()}
	<Button
		tier="primary"
		icon="file-down"
		onclick={download}
		disabled={blocked !== null}
		reason={blocked ?? undefined}
	>
		{working ? 'Building…' : 'Download CSV'}
	</Button>
{/snippet}

<div class="page flow-page">
	<PageHead
		icon="file-down"
		title="Export"
		description="Download your resources as a spreadsheet."
		guide="export"
	/>

	<Stepper {steps} label="Export steps" />

	<div class="flow">
		<FlowStep
			n={1}
			id="what"
			title="What"
			hint="Choose what goes in the spreadsheet."
			summary={whatLine}
			done={what === 'all' || collection !== ''}
			bind:open={whatOpen}
		>
			<div class="flow-choice" role="radiogroup" aria-label="What to export">
				<button type="button" role="radio" aria-checked={what === 'all'} onclick={() => (what = 'all')}>
					Every resource
					<span class="sub">Your whole catalogue.</span>
				</button>
				<button
					type="button"
					role="radio"
					aria-checked={what === 'collection'}
					disabled={collectionRefusal !== null}
					title={collectionRefusal ?? undefined}
					onclick={() => (what = 'collection')}
				>
					One collection
					<span class="sub">{collectionRefusal ?? 'In the collection’s order.'}</span>
				</button>
			</div>

			{#if what === 'collection' && collectionRefusal === null}
				<Field label="Collection" id="export-collection">
					<select id="export-collection" bind:value={collection} disabled={collections.isPending}>
						<option value="">
							{collections.isPending ? 'Loading your collections…' : 'Choose a collection'}
						</option>
						{#each heldCollections as one (one.id)}
							<option value={one.id}>{one.name} ({one.count})</option>
						{/each}
					</select>
				</Field>
			{/if}
		</FlowStep>

		<FlowStep
			n={2}
			id="format"
			title="Format"
			hint="The spreadsheet opens in Excel, Numbers or Google Sheets."
			summary="CSV"
			done
			bind:open={formatOpen}
		>
			<div class="flow-choice" role="radiogroup" aria-label="Format">
				<button type="button" role="radio" aria-checked="true">
					CSV spreadsheet
					<span class="sub">One row per resource.</span>
				</button>
			</div>
		</FlowStep>

		<FlowStep
			n={3}
			id="download"
			title="Download"
			hint="Press Download; the file goes to your downloads folder."
			done={outcome.kind === 'handed'}
			bind:open={downloadOpen}
			footer={downloadButton}
		>
			{#snippet aside()}
				<Explain title="What the spreadsheet holds" label="What’s in it?">
					<p>One row per resource: title, price, currency, labels, and when it was created and last changed.</p>
					<p>
						For every marketplace — TES, TPT and Etsy — that listing’s status, the price you set
						there, and a link to the live page.
					</p>
					<p>Your resource files are not included, and neither are your marketplace logins.</p>
					<p>Each file is named <span class="export-file">{FILENAME_PATTERN}</span>, dated in UTC.</p>
				</Explain>
			{/snippet}

			<p class="export-lede">
				<strong>{whatLine}</strong> as CSV.
			</p>

			{#if empty}
				<p class="export-empty">{NOTHING_TO_EXPORT}</p>
			{/if}

			<div class="export-outcome" aria-live="polite">
				{#if working}
					<p class="export-working" role="status">
						<span class="export-pulse" aria-hidden="true"></span>
						Building your spreadsheet…
					</p>
				{:else if outcome.kind === 'handed'}
					<Banner tone="ok" title="Your spreadsheet is ready">
						Downloading as <span class="export-file">{outcome.filename}</span>.
						{HANDOFF_ADVICE}
					</Banner>
				{:else if outcome.kind === 'failed'}
					<Banner
						tone="bad"
						title="The export did not finish"
						action={outcome.signIn ? signIn : undefined}
					>
						{outcome.message}
					</Banner>
				{/if}
			</div>
		</FlowStep>
	</div>
</div>
