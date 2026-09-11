<script lang="ts">
	import type { Snippet } from 'svelte';
	import Icon from '$lib/Icon.svelte';
	import type { IconName } from '$lib/icons';
	import {
		GROUP_HEADINGS,
		refusalsIn,
		type Advisory,
		type FormAnchor,
		type Refusal
	} from '$lib/tpt-form';

	let {
		group,
		icon,
		mark,
		help,
		refusals = [],
		advisories = [],
		badge,
		children
	}: {
		group: FormAnchor;
		icon?: IconName;
		/** A marketplace's own mark in place of an icon, which is how the two
		 *  per-marketplace panels say whose options they hold: a teacher
		 *  recognises the logo faster than the words beside it. */
		mark?: string;
		help?: string;
		refusals?: readonly Refusal[];
		advisories?: readonly Advisory[];
		/** Drawn on the heading line, after the heading. For a note about the
		 *  band as a whole that is not an instruction: help is a sentence
		 *  telling the teacher what to do, and this is not one. */
		badge?: Snippet;
		children: Snippet;
	} = $props();

	const heading = $derived(GROUP_HEADINGS[group]);
	const mine = $derived(refusalsIn(refusals, group));
	const said = $derived(advisories.filter((advisory) => advisory.group === group));
</script>

<!-- One band of the form, with its heading, its one sentence of help and the
     refusals that belong to it. The heading is the segregation: a teacher
     looking for the tax code looks under TPT only because that is the
     marketplace that asks for it, and a refusal about a picker appears under
     the picker rather than in one list at the foot of the page.

     A band rather than a card: a dozen cards abutting inside one form read as
     one white slab seamed by a shadow, so the sections are separated by a
     hairline and the card is the surface that holds them all. -->
<section class="res-sec" id="group-{group}" aria-labelledby="heading-{group}">
	<div class="res-sec-h">
		{#if mark}
			<img class="res-sec-mark" src={mark} alt="" />
		{:else if icon}
			<span class="res-sec-ico"><Icon name={icon} size={16} /></span>
		{/if}
		<h2 id="heading-{group}">{heading}</h2>
		{#if badge}{@render badge()}{/if}
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
