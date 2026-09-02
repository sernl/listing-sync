<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { FormGroup } from '$lib/generated/vocab';
	import { GROUP_HEADINGS, refusalsIn, type Advisory, type Refusal } from '$lib/tpt-form';

	let {
		group,
		help,
		refusals = [],
		advisories = [],
		children
	}: {
		group: FormGroup;
		help?: string;
		refusals?: readonly Refusal[];
		advisories?: readonly Advisory[];
		children: Snippet;
	} = $props();

	const heading = $derived(GROUP_HEADINGS[group]);
	const mine = $derived(refusalsIn(refusals, group));
	const said = $derived(advisories.filter((advisory) => advisory.group === group));
</script>

<!-- One of TPT's own nine sections, with its heading, its helper text and the
     refusals that belong to it. The heading is the segregation: a seller
     looking for the tax code looks under Price because that is where TPT puts
     it, and a refusal about a picker appears under the picker rather than in
     one list at the foot of the page. -->
<section class="panel form-group" id="group-{group}" aria-labelledby="heading-{group}">
	<div class="head-row">
		<div>
			<h2 id="heading-{group}">{heading}</h2>
			{#if help}<div class="desc">{help}</div>{/if}
		</div>
	</div>
	{@render children()}
	{#if mine.length > 0}
		<ul class="refusals">
			{#each mine as refusal, index (`${refusal.control ?? group}-${index}`)}
				<li class="blocking">{refusal.message}</li>
			{/each}
		</ul>
	{/if}
	{#each said as advisory, index (index)}
		<p class="disclosure">{advisory.message}</p>
	{/each}
</section>
