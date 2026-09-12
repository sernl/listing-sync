<script lang="ts">
	import type { Snippet } from 'svelte';
	import { accountTileState } from '$lib/account-tile.svelte';
	import Icon from '$lib/Icon.svelte';
	import type { IconName } from '$lib/icons';
	import { ACCOUNT_DESTINATION } from '$lib/nav';

	let {
		icon,
		title,
		description,
		back,
		search,
		aside
	}: {
		icon: IconName;
		title: string;
		description: string;
		/** Where the back control goes on a drill-down view. Set only there:
		 *  Vendoo's own pattern replaces the icon with a back arrow and drops
		 *  the help control, because a drill-down is reached from one page and
		 *  returns to it. */
		back?: { href: string; label: string };
		/** Opens this page's search. Set by the page that has one -- Resources
		 *  -- and drawn only on a phone, where the tab bar carries no search
		 *  cell and the top strip is not drawn: below 620px this button and
		 *  Ctrl-K are the two ways to the palette. */
		search?: () => void;
		aside?: Snippet;
	} = $props();

	const tile = $derived(accountTileState.tile);
</script>

<header class="page-head">
	{#if back}
		<a class="page-ico" href={back.href} aria-label={back.label}>
			<Icon name="arrow-left" size={18} />
		</a>
	{:else}
		<div class="page-ico"><Icon name={icon} size={18} /></div>
	{/if}
	<div class="head-titles">
		<h1>{title}</h1>
		<p>{description}</p>
	</div>
	{#if aside}
		<div class="head-aside">{@render aside()}</div>
	{/if}
	<!-- Rendered here rather than by each page, so every header band carries
	     exactly one and no page has to remember. A drill-down has none: it is
	     reached from one screen and returns to it, which is Vendoo's own
	     pattern and the specification's. -->
	{#if !back}
		<a class="page-help" href="/guides" aria-label="Help with this page">
			<Icon name="circle-question-mark" size={20} />
		</a>
		<!-- The phone's own two controls, which `app.css` draws only below
		     620px. Above it the top strip carries the search box and the
		     account tile and the rail carries the section, so these would be a
		     second copy of both. They sit in the header band rather than on the
		     tab bar because the founder's 2026-09-12 review took search and
		     Account off a seven-cell bar: search is a task rather than a
		     destination, and Account is secondary administration. -->
		<div class="head-phone">
			{#if search}
				<button class="head-tool" type="button" aria-label="Search resources" onclick={search}>
					<Icon name="search" size={20} />
				</button>
			{/if}
			<!-- Named rather than described by its contents: the initials are
			     decoration, so without this label the link announces as the
			     organisation's name rather than as Account. The section's own
			     glyph stands in while the tile knows neither a picture nor a
			     name -- every frame before the reads land -- because an empty
			     tile reads as a loading state that never resolves and the glyph
			     reads as Account, which is true in every state. -->
			<a
				class="head-tool head-account"
				href={ACCOUNT_DESTINATION.href}
				aria-label={ACCOUNT_DESTINATION.label}
			>
				{#if tile.kind === 'picture'}
					<img
						class="avatar-pic"
						src={tile.src}
						alt=""
						onerror={() => accountTileState.unusable()}
					/>
				{:else if tile.kind === 'initials'}
					<span aria-hidden="true">{tile.text}</span>
				{:else}
					<Icon name={ACCOUNT_DESTINATION.icon} size={20} />
				{/if}
			</a>
		</div>
	{/if}
</header>
