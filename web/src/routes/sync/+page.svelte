<script lang="ts">
	import { api, type JobHead } from '$lib/api';
	import { agoLabel } from '$lib/elapsed';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';

	let jobs = $state<JobHead[]>([]);
	let nextCursor = $state<string | null>(null);
	let loaded = $state(false);

	async function loadPage(cursor?: string | null) {
		const view = await api.jobs(cursor);
		jobs = [...jobs, ...view.jobs];
		nextCursor = view.next_cursor;
		loaded = true;
	}

	$effect(() => {
		void loadPage();
	});
</script>

<div class="page">
	<PageHead
		icon="⇄"
		title="Sync"
		description="Queued and completed runs, with live progress while one is under way."
	/>

	<Panel>
		{#if !loaded}
			<p class="quiet">Loading…</p>
		{:else if jobs.length === 0}
			<div class="placeholder">
				<span class="big" aria-hidden="true">⇄</span>
				<b>No sync has run yet.</b>
				<p>
					Start one from Listings: choose the resources to send, and the engine takes them
					from there. Every run keeps its own record here.
				</p>
			</div>
		{:else}
			{#each jobs as job (job.job)}
				<a class="job" href={`/sync/${job.job}`}>
					<span class="badge">{job.inventory}</span>
					<span class="what">
						<span class="t mono">{job.job}</span>
						<span class="w">{new Date(job.created_at).toLocaleString()}</span>
					</span>
					<span class="when">{agoLabel(job.created_at, Date.now())}</span>
				</a>
			{/each}
			{#if nextCursor}
				<div class="actions">
					<button class="btn" onclick={() => loadPage(nextCursor)}>Load more</button>
				</div>
			{/if}
		{/if}
	</Panel>
</div>
