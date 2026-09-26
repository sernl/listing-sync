<script lang="ts">
	// Mappings: the rules that say which of the target marketplace's own terms —
	// its licence and its resource type — a resource should land under, and the
	// proposals a seller approves before any of them is used.

	import PageHead from '$lib/PageHead.svelte';
	import RuleWorkbench from '$lib/pages/automations/RuleWorkbench.svelte';
	import { MAPPING_GUIDE, WHAT_MAPPING_IS } from '$lib/pages/automations/seller-rules';
	import '$lib/pages/automations/automations.css';
	import '$lib/flow.css';
</script>

<div class="page flow-page has-bar">
	<PageHead
		icon="tag"
		title="Target terms"
		description={WHAT_MAPPING_IS}
		guide={MAPPING_GUIDE}
	/>

	<RuleWorkbench kind="mapping" />
</div>
