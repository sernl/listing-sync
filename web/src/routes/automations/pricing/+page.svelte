<script lang="ts">
	// Pricing: the rules that say what a resource costs on the marketplace it is
	// sent to, and the proposals a seller approves before any of them is used.
	//
	// Thin, like every other route here: the surface is
	// `$lib/pages/automations/RuleWorkbench.svelte`, and the words and refusals
	// it renders are in `$lib/pages/automations/seller-rules`.

	import PageHead from '$lib/PageHead.svelte';
	import RuleWorkbench from '$lib/pages/automations/RuleWorkbench.svelte';
	import { PRICING_GUIDE, WHAT_PRICING_IS } from '$lib/pages/automations/seller-rules';
	import '$lib/pages/automations/automations.css';
</script>

<div class="page">
	<!-- The lead sentence is the header's own description rather than a
	     paragraph under it: two channels saying the same thing is the second
	     one a seller stops reading. -->
	<PageHead
		icon="credit-card"
		title="Pricing"
		description={WHAT_PRICING_IS}
		guide={PRICING_GUIDE}
	/>

	<RuleWorkbench kind="pricing" />
</div>
