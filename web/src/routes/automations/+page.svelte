<script lang="ts">
	import { api, type ConnectionView, type SyncRequestHead } from '$lib/api';
	import { sellerRules, type RuleCounts } from '$lib/seller-rules';
	import Button from '$lib/Button.svelte';
	import Icon from '$lib/Icon.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import StatusPill from '$lib/StatusPill.svelte';
	import { cards } from '$lib/pages/automations/landing';
	import '$lib/flow.css';

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

<div class="page flow-page">
	<PageHead
		icon="waves-horizontal"
		title="Automations"
		description="Move, price and publish your resources, then keep them up to date."
	/>

	<!-- One path, in the order a seller meets the five: each stop is a card
	     with its number, what it does in one sentence, and where it stands. -->
	<ol class="journey">
		{#each shown as card, index (card.id)}
			<li class="stop">
				<span class="stop-num" aria-hidden="true">{index + 1}</span>
				<a class="stop-card" href={card.href}>
					<span class="stop-ring"><Icon name={card.icon} size={20} /></span>
					<span class="stop-main">
						<span class="stop-title">{card.title}</span>
						<span class="stop-what">{card.what}</span>
					</span>
					<span class="stop-state">
						<StatusPill tone={card.state.tone} label={card.state.label} />
					</span>
					<span class="stop-go" aria-hidden="true"><Icon name="chevron-right" size={18} /></span>
				</a>
			</li>
		{/each}
	</ol>

	<div class="journey-foot">
		<Button href="/automations/migration" tier="primary" icon="arrow-right-left">
			Start with a migration
		</Button>
	</div>
</div>

<style>
	.journey {
		display: flex;
		flex-direction: column;
		gap: var(--s-5);
		margin: 0;
		padding: 0;
		list-style: none;
		position: relative;
	}

	/* The path: one line behind the numbers, from the first to the last. */
	.journey::before {
		content: '';
		position: absolute;
		left: 17px;
		top: 24px;
		bottom: 24px;
		width: 2px;
		border-radius: 1px;
		background: linear-gradient(var(--lavender), var(--accent));
	}

	.stop {
		position: relative;
		display: grid;
		grid-template-columns: 36px minmax(0, 1fr);
		align-items: center;
		gap: var(--s-4);
	}

	.stop-num {
		z-index: 1;
		display: grid;
		place-items: center;
		width: 36px;
		height: 36px;
		border-radius: 50%;
		background: var(--surface);
		border: 2px solid var(--lavender);
		color: var(--additive);
		font-family: var(--display);
		font-weight: 600;
	}

	.stop-card {
		display: grid;
		grid-template-columns: auto minmax(0, 1fr) auto auto;
		align-items: center;
		gap: var(--s-4);
		padding: var(--s-4) var(--s-5);
		border: 1px solid var(--line);
		border-radius: var(--r-card);
		background: var(--card);
		box-shadow: var(--sh-1);
		color: var(--text);
		text-decoration: none;
		transition:
			border-color 0.15s ease,
			box-shadow 0.15s ease;
	}

	.stop-card:hover {
		border-color: var(--lavender);
		box-shadow: var(--sh-2);
	}

	.stop-card:focus-visible {
		outline: 2px solid var(--accent);
		outline-offset: 2px;
	}

	.stop-ring {
		display: grid;
		place-items: center;
		width: 44px;
		height: 44px;
		border-radius: 12px;
		background: var(--additive-soft);
		color: var(--additive);
	}

	.stop-main {
		display: flex;
		flex-direction: column;
		gap: 2px;
		min-width: 0;
	}

	.stop-title {
		font-family: var(--display);
		font-size: 17px;
		font-weight: 600;
	}

	.stop-what {
		color: var(--muted);
		font-size: 13.5px;
	}

	.stop-go {
		display: inline-flex;
		color: var(--muted);
	}

	.journey-foot {
		margin-top: var(--s-6);
		padding-left: 52px;
	}

	@media (max-width: 720px) {
		.stop {
			gap: var(--s-3);
		}

		.stop-card {
			grid-template-columns: auto minmax(0, 1fr) auto;
			padding: var(--s-4);
		}

		.stop-state {
			grid-column: 2 / -1;
			grid-row: 2;
		}

		.stop-go {
			grid-column: 3;
			grid-row: 1;
		}

		.stop-ring {
			width: 38px;
			height: 38px;
		}

		.journey-foot {
			padding-left: 0;
		}
	}
</style>
