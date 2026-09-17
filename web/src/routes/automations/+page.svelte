<script lang="ts">
	import { api, type ConnectionView, type SyncRequestHead } from '$lib/api';
	import { sellerRules, type RuleCounts } from '$lib/seller-rules';
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
	// The rule totals the Pricing and Mappings cards state. Null until read,
	// for the same reason: a card that says "no rule yet" about a read that
	// failed is a card that lies about the seller's own work.
	let pricingRules = $state<RuleCounts | null>(null);
	let mappingRules = $state<RuleCounts | null>(null);

	$effect(() => {
		void api
			.connections()
			.then((held) => (connections = held))
			.catch(() => (connections = []));
		void api
			.syncRequests()
			.then((view) => (requests = view.requests))
			.catch(() => (requests = []));
		void api
			.drainStats()
			.then((stats) => (openQuestions = stats.open))
			.catch(() => (openQuestions = null));
		void sellerRules
			.list({ kind: 'pricing', state: 'all' })
			.then((view) => (pricingRules = view.counts))
			.catch(() => (pricingRules = null));
		void sellerRules
			.list({ kind: 'mapping', state: 'all' })
			.then((view) => (mappingRules = view.counts))
			.catch(() => (mappingRules = null));
	});

	const shown = $derived(
		cards({ connections, requests, openQuestions, pricingRules, mappingRules })
	);
</script>

<div class="page">
	<PageHead
		icon="waves-horizontal"
		title="Automations"
		description="Edit tags, descriptions, titles and files across your listings."
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
