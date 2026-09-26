<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { api } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import Note from '$lib/Note.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import { EMPTINESS_KEY, fetchCatalogueCsv } from '$lib/pages/export/api';
	import {
		FILENAME_PATTERN,
		HANDOFF_ADVICE,
		runExport,
		saveDocument,
		type ExportState
	} from '$lib/pages/export/download';
	import '$lib/pages/export/export.css';

	// One page, not the whole catalogue walk: this asks only whether anything
	// exists, and the first page answers that exactly.
	const catalogue = createQuery(() => ({
		queryKey: EMPTINESS_KEY,
		queryFn: () => api.products()
	}));

	// Fail open. A catalogue that could not be counted is not one known to be
	// empty, and withholding this page's only action over a question the
	// seller never asked would be the worse answer of the two.
	const empty = $derived(catalogue.isSuccess && catalogue.data.products.length === 0);
	const NOTHING_TO_EXPORT = 'There is nothing to export yet.';

	let state = $state<ExportState>({ kind: 'idle' });
	const working = $derived(state.kind === 'working');

	async function download() {
		// The button stays focusable while the export runs, so a second press
		// is refused here rather than by removing the control the seller is
		// standing on.
		if (working) {
			return;
		}
		state = { kind: 'working' };
		state = await runExport(() => fetchCatalogueCsv(), saveDocument);
	}
</script>

{#snippet signIn()}
	<Button href="/login">Sign in</Button>
{/snippet}

<div class="page">
	<PageHead
		icon="file-down"
		title="Export"
		description="A spreadsheet of your resources, with each marketplace's status, price and link."
		guide="export"
	/>

	<Panel title="Export your resources">
		<p class="export-lede">One row per resource. Your resource files are not included.</p>

		<dl class="export-carries">
			<div>
				<dt>Every resource</dt>
				<dd>Title, price, currency, labels, and when it was created and last changed.</dd>
			</div>
			<div>
				<dt>Every marketplace</dt>
				<dd>
					TES, TPT and Etsy, each with that listing's status, the price you set there, and a
					link to the live page.
				</dd>
			</div>
		</dl>

		<div class="export-actions">
			<Button
				tier="primary"
				icon="file-down"
				onclick={download}
				disabled={empty}
				reason={empty ? NOTHING_TO_EXPORT : undefined}
			>
				Export CSV
			</Button>
			{#if empty}
				<p class="export-empty">{NOTHING_TO_EXPORT}</p>
			{/if}
		</div>

		<div class="export-outcome" aria-live="polite">
			{#if working}
				<p class="export-working" role="status">
					<span class="export-pulse" aria-hidden="true"></span>
					Building your spreadsheet…
				</p>
			{:else if state.kind === 'handed'}
				<Banner tone="ok" title="Your spreadsheet is ready">
					Downloading as <span class="export-file">{state.filename}</span>.
					{HANDOFF_ADVICE}
				</Banner>
			{:else if state.kind === 'failed'}
				<Banner
					tone="bad"
					title="The export did not finish"
					action={state.signIn ? signIn : undefined}
				>
					{state.message}
				</Banner>
			{/if}
		</div>

		<Note>
			Each file is named <span class="export-file">{FILENAME_PATTERN}</span>, dated in UTC.
		</Note>
	</Panel>
</div>
