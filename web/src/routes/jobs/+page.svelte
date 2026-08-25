<script lang="ts">
	import { api, type JobHead } from '$lib/api';

	let jobs = $state<JobHead[]>([]);
	let nextCursor = $state<string | null>(null);
	let loaded = $state(false);

	async function loadPage(cursor?: string | null) {
		const page = await api.jobs(cursor);
		jobs = [...jobs, ...page.jobs];
		nextCursor = page.next_cursor;
		loaded = true;
	}

	$effect(() => {
		void loadPage();
	});
</script>

<h1 class="mb-4 text-xl font-semibold">Jobs</h1>

{#if !loaded}
	<p class="text-slate-500">Loading…</p>
{:else if jobs.length === 0}
	<p class="text-slate-500">No jobs yet; start one from the products table.</p>
{:else}
	<ul class="divide-y divide-slate-100 rounded border border-slate-200 bg-white">
		{#each jobs as job (job.job)}
			<li>
				<a class="flex items-center gap-4 px-4 py-3 hover:bg-slate-50" href={`/jobs/${job.job}`}>
					<span class="font-mono text-xs text-slate-500">{job.job.slice(0, 8)}…</span>
					<span class="rounded bg-slate-100 px-2 py-0.5 text-xs">{job.inventory}</span>
					<span class="text-sm text-slate-500">
						{new Date(job.created_at).toLocaleString()}
					</span>
				</a>
			</li>
		{/each}
	</ul>
	{#if nextCursor}
		<button
			class="mt-3 rounded border border-slate-300 px-3 py-1 text-sm"
			onclick={() => loadPage(nextCursor)}
		>
			Load more
		</button>
	{/if}
{/if}
