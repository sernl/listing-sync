<script lang="ts">
	import { api, type ConnectionView, type SyncRequestHead } from '$lib/api';
	import Button from '$lib/Button.svelte';
	import Icon from '$lib/Icon.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import StatusPill from '$lib/StatusPill.svelte';
	import { cards } from '$lib/pages/automations/landing';
	import '$lib/pages/automations/automations.css';

	let connections = $state<ConnectionView[]>([]);
	let requests = $state<SyncRequestHead[]>([]);
	// Null until the figure has actually been read. A count this page failed to
	// fetch is not a count of zero, and the sync card says so by dropping the
	// figure rather than by claiming one.
	let openQuestions = $state<number | null>(null);

	$effect(() => {
		void api
			.connections()
			.then((view) => (connections = view.connections))
			.catch(() => (connections = []));
		void api
			.syncRequests()
			.then((view) => (requests = view.requests))
			.catch(() => (requests = []));
		void api
			.drainStats()
			.then((stats) => (openQuestions = stats.open))
			.catch(() => (openQuestions = null));
	});

	const shown = $derived(cards({ connections, requests, openQuestions }));
</script>

<div class="page">
	<PageHead
		icon="waves-horizontal"
		title="Automations"
		description="The three things Teachouse can keep doing for you on a schedule."
	/>

	<div class="auto-cards">
		{#each shown as card (card.id)}
			<article class="auto-card">
				<div class="card-head">
					<span class="ring"><Icon name={card.icon} size={19} /></span>
					<h2>{card.title}</h2>
				</div>
				<p>{card.what}</p>
				<div class="go">
					<StatusPill tone={card.state.tone} label={card.state.label} />
					<Button href={card.href} tier="outline">Open</Button>
				</div>
			</article>
		{/each}
	</div>
</div>
