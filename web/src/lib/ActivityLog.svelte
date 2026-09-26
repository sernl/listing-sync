<script lang="ts" module>
	export interface LogEntry {
		id: string;
		/** What happened, in one line and in the seller's words. */
		what: string;
		/** When, already formatted: this component does no clock arithmetic. */
		at: string;
		/** Where the line's subject is, where the log's source composed one.
		 *  Absent on a log whose lines name nothing openable. */
		href?: string;
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

	const needle = $derived(query.trim().toLowerCase());
	const shown = $derived(
		needle === '' ? entries : entries.filter((entry) => entry.what.toLowerCase().includes(needle))
	);
</script>

<div class="log-head">
	<span class="log-search">
		<Icon name="search" size={15} />
		<label class="sr-only" for="log-search">Search this page</label>
		<!-- "on this page" in the label and the placeholder, and not by
		     accident. The lines are worded by the server from three different
		     event kinds, so a search the server could answer would have to
		     match structured rows rather than the sentences the seller is
		     reading — and the log is paged. A box that said "Search this log"
		     would answer about the page in hand while looking like it had
		     searched the history, which is the one thing a history must not
		     do. Older lines are reached with Previous and Next. -->
		<input
			id="log-search"
			type="search"
			placeholder="Search this page…"
			bind:value={query}
		/>
	</span>
	<!-- The page's own figure, said as the page's: no total is claimed,
	     because the page length is not one and a count drawn from it would be
	     a history's size invented from a screenful. -->
	<span class="log-count" role="status" aria-live="polite">
		{needle === ''
			? `${entries.length} on this page`
			: `${shown.length} of ${entries.length} on this page`}
	</span>
</div>

{#if shown.length === 0}
	<p class="quiet">
		{entries.length === 0 ? empty : 'Nothing on this page matches. Clear the search, or use Previous and Next.'}
	</p>
{:else}
	{#each shown as entry (entry.id)}
		<div class="log-row">
			{#if entry.href}
				<a class="what" href={entry.href}>{entry.what}</a>
			{:else}
				<span class="what">{entry.what}</span>
			{/if}
			<span class="at">{entry.at}</span>
		</div>
	{/each}
{/if}
