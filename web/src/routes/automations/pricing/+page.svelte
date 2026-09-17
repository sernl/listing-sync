<script lang="ts">
	// Pricing: the rules that say what a resource costs on the marketplace it is
	// sent to, and the proposals a seller approves before any of them is used.
	//
	// Thin, like every other route here: the surface is
	// `$lib/pages/automations/RuleWorkbench.svelte`, and the words and refusals
	// it renders are in `$lib/pages/automations/seller-rules`.

	import PageHead from '$lib/PageHead.svelte';
	import RuleWorkbench from '$lib/pages/automations/RuleWorkbench.svelte';
	import { WHAT_PRICING_IS } from '$lib/pages/automations/seller-rules';
	import '$lib/pages/automations/automations.css';
</script>

<div class="page">
	<PageHead
		icon="credit-card"
		title="Pricing"
		description="Convert your prices for another marketplace, and approve every figure before it is used."
	/>

	<p class="migrate-lead">{WHAT_PRICING_IS}</p>

	<RuleWorkbench kind="pricing" />
</div>
