<script lang="ts">
	import type { Snippet } from 'svelte';
	import { accountTileState } from '$lib/account-tile.svelte';
	import Icon from '$lib/Icon.svelte';
	import type { IconName } from '$lib/icons';
	import { ACCOUNT_DESTINATION } from '$lib/nav';
	import { whereYouAre } from '$lib/machine-here';
	import { machineHere } from '$lib/machine.svelte';

	let {
		icon,
		title,
		description,
		guide,
		back,
		search,
		aside
	}: {
		icon: IconName;
		title: string;
		/** The one sentence under the title, where the title alone does not
		 *  say what to do here. Optional: a page whose name is its own
		 *  instruction says nothing twice. */
		description?: string;
		/** The guide this page's help control opens, by slug. Without one the
		 *  control opens the guide index, which is where it went before any
		 *  page named its own. */
		guide?: string;
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
		{#if description}<p>{description}</p>{/if}
	</div>
	{#if aside}
		<div class="head-aside">{@render aside()}</div>
	{/if}
	<!-- Rendered here rather than by each page, so every header band carries
	     exactly one and no page has to remember. A drill-down carries none of
	     its own accord -- it is reached from one screen and returns to it,
	     which is Vendoo's own pattern and the specification's -- but a
	     drill-down that names its guide gets the control anyway, because
	     naming one is the page asking for it. -->
	{#if !back || guide !== undefined}
		<a
			class="page-help"
			href={guide === undefined ? '/guides' : `/guides/${guide}`}
			aria-label={guide === undefined ? 'Help with this page' : 'Read the guide'}
		>
			<Icon name="circle-question-mark" size={20} />
		</a>
	{/if}
	{#if !back}
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
			<!-- The title says which machine the seller is at and whether they are
			     in the app, which on a phone is the only place the strip's own
			     line could go: the top strip is not drawn below 620px at all. It
			     is a supplement to the label rather than a replacement, so the
			     control still announces as Account. -->
			<a
				class="head-tool head-account"
				href={ACCOUNT_DESTINATION.href}
				aria-label={ACCOUNT_DESTINATION.label}
				title={whereYouAre(machineHere.where)}
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
