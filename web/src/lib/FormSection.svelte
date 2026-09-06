<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { FormGroup } from '$lib/generated/vocab';
	import Icon from '$lib/Icon.svelte';
	import type { IconName } from '$lib/icons';
	import { GROUP_HEADINGS, refusalsIn, type Advisory, type Refusal } from '$lib/tpt-form';

	let {
		group,
		icon,
		help,
		refusals = [],
		advisories = [],
		children
	}: {
		group: FormGroup;
		icon?: IconName;
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
     one list at the foot of the page.

     A band rather than a card: nine cards abutting inside one form read as one
     white slab seamed by a shadow, so the sections are separated by a hairline
     and the card is the surface that holds them all. -->
<section class="res-sec" id="group-{group}" aria-labelledby="heading-{group}">
	<div class="res-sec-h">
		{#if icon}<span class="res-sec-ico"><Icon name={icon} size={16} /></span>{/if}
		<h2 id="heading-{group}">{heading}</h2>
	</div>
	{#if help}<p class="res-sec-help">{help}</p>{/if}
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
