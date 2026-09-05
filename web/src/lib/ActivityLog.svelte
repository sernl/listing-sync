<script lang="ts" module>
	export interface LogEntry {
		id: string;
		/** What happened, in one line and in the seller's words. */
		what: string;
		/** When, already formatted: this component does no clock arithmetic. */
		at: string;
	}
</script>

<script lang="ts">
	import Icon from '$lib/Icon.svelte';

	let {
		entries,
		query = $bindable(''),
		empty = 'Nothing has happened here yet.'
	}: {
		entries: readonly LogEntry[];
		query?: string;
		empty?: string;
	} = $props();

	const shown = $derived(
		query.trim() === ''
			? entries
			: entries.filter((entry) => entry.what.toLowerCase().includes(query.trim().toLowerCase()))
	);
</script>

<div class="log-head">
	<span class="log-search">
		<Icon name="search" size={15} />
		<label class="sr-only" for="log-search">Search this log</label>
		<input id="log-search" type="search" placeholder="Search this log…" bind:value={query} />
	</span>
	<span class="log-count">Viewing ({shown.length})</span>
</div>

{#if shown.length === 0}
	<p class="quiet">{empty}</p>
{:else}
	{#each shown as entry (entry.id)}
		<div class="log-row">
			<span class="what">{entry.what}</span>
			<span class="at">{entry.at}</span>
		</div>
	{/each}
{/if}
