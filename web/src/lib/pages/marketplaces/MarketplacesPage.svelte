<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { api } from '$lib/api';
	import AddCard from '$lib/AddCard.svelte';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { currentSessionToken, listBrowserSessions } from '$lib/browser-sessions';
	import { merge } from '$lib/device-merge';
	import { marketplaceRows, needingAttention } from '$lib/devices-view';
	import { TRANSPORT_OF } from '$lib/inventory';
	import { queryKeys } from '$lib/query';
	import Authorship from './Authorship.svelte';
	import Downloads from './Downloads.svelte';
	import Machines from './Machines.svelte';
	import MarketplaceCard from './MarketplaceCard.svelte';
	import RequestCard from './RequestCard.svelte';
	import { fetchManifest } from './api';
	import { DISCLAIMER, EXTENSIONS, LISTED, LIVE, PLANNED } from './catalogue';
	import {
		CARD_NAME,
		type LiveRead,
		TRANSPORT_BADGE,
		deviceBranchInTileOrder,
		liveFace,
		transportLine
	} from './view';
	import './marketplaces.css';

	const registry = createQuery(() => ({ queryKey: queryKeys.devices, queryFn: () => api.devices() }));
	const linked = createQuery(() => ({
		queryKey: queryKeys.connections,
		queryFn: () => api.connections()
	}));
	const signIns = createQuery(() => ({
		queryKey: queryKeys.browserSessions,
		queryFn: () => listBrowserSessions()
	}));
	const current = createQuery(() => ({
		queryKey: queryKeys.currentSessionToken,
		queryFn: () => currentSessionToken()
	}));
	const releases = createQuery(() => ({
		queryKey: queryKeys.downloadsManifest,
		queryFn: () => fetchManifest()
	}));

	// Read once when the page renders rather than per row, so every age on it is
	// measured from the same instant and the list does not appear to tick.
	const now = Date.now();

	let requesting = $state(false);

	const devices = $derived(registry.data?.devices ?? []);
	const connections = $derived(linked.data?.connections ?? []);
	const rows = $derived(marketplaceRows(devices, connections, now));
	const joined = $derived(merge(devices, signIns.data ?? [], current.data ?? null));

	/** Whether the two reads this page's live tiles depend on have landed.
	 *
	 *  Both, because a tile needs the device registry and the connection list to
	 *  say anything: either one missing leaves the answer unknown rather than
	 *  partly known. */
	const read = $derived<LiveRead['state']>(
		registry.isPending || linked.isPending
			? 'pending'
			: registry.isError || linked.isError
				? 'failed'
				: 'read'
	);

	/** The two connected marketplaces, in the catalogue's order rather than the
	 *  device view's: this page leads with them, and the order they are tiled in
	 *  is a decision of the page's own.
	 *
	 *  Built from `LIVE` rather than from `rows`, so both tiles are on the page
	 *  before any fetch resolves and stay there if one fails. A marketplace the
	 *  seller sells on must not vanish from a page titled "Every marketplace you
	 *  sell on" because a read did not land, and the grid must not reflow around
	 *  two tiles arriving late. */
	const liveCards = $derived(
		LIVE.map((tile) => {
			const row = rows.find((entry) => entry.marketplace === tile.marketplace);
			const held: LiveRead =
				read === 'read' && row !== undefined ? { state: 'read', row } : { state: read === 'failed' ? 'failed' : 'pending' };
			return { tile, face: liveFace(held) };
		})
	);

	/** Only the marketplaces this page offers an action for.
	 *
	 *  `needingAttention` answers over every marketplace the platform knows,
	 *  Etsy included, and Etsy is tiled here as one we have not built: naming it
	 *  in a banner would ask the seller to attend to something they cannot act
	 *  on. */
	const waiting = $derived(
		read === 'read'
			? needingAttention(rows).filter((row) =>
					LIVE.some((tile) => tile.marketplace === row.marketplace)
				)
			: []
	);

	/** The marketplaces a copyright declaration is made for, in the order the
	 *  grid above tiles them, so one screen does not list one pair two ways. */
	const declarable = $derived(
		read === 'read'
			? deviceBranchInTileOrder(
					rows,
					LIVE.map((tile) => tile.marketplace)
				)
			: []
	);

	/** Whether TPT is waiting on a copyright declaration.
	 *
	 *  Its own banner rather than a line on the card, because it is the one
	 *  thing on this page that silently fails a send: TPT refuses a listing that
	 *  does not name who holds the copyright, and the refusal is not retried.
	 *
	 *  Only once the read lands. `marketplaceRows` maps a total order, so it
	 *  answers a row per marketplace whatever the fetch did, and an unread row
	 *  carries no authorship — which is indistinguishable from a seller who has
	 *  not declared. Raising the banner on that would tell a seller their TPT
	 *  sends are failing on a fact we never received. */
	const undeclared = $derived(
		read === 'read' &&
			rows.some(
				(row) =>
					row.marketplace === 'Tpt' &&
					(row.authorship === undefined || row.authorship.state === 'undeclared')
			)
	);
</script>

<div class="page">
	<div class="mp-title">
		<div>
			<h1>Marketplaces</h1>
			<p>
				Every marketplace you sell on, where its login is kept, and whether it can be used
				right now.
			</p>
		</div>
		<span class="act">
			<Button tier="primary" icon="circle-plus" href="#downloads">Connect a marketplace</Button>
		</span>
	</div>

	{#if waiting.length > 0}
		<Banner
			tone="warn"
			title="{waiting.length} {waiting.length === 1 ? 'marketplace needs' : 'marketplaces need'} you"
		>
			{waiting.map((row) => CARD_NAME[row.marketplace]).join(', ')}.
		</Banner>
	{/if}

	{#if undeclared}
		<Banner tone="warn" title="TPT has no copyright declaration" action={toCopyright}>
			TPT needs every listing to name who holds the copyright. Until you declare it, anything
			sent to TPT fails and is not tried again.
		</Banner>
	{/if}

	<div class="mp-grid">
		{#each liveCards as card (card.tile.marketplace)}
			<MarketplaceCard
				mark={card.tile.mark}
				name={card.tile.name}
				home={card.tile.home}
				handle={card.face.handle}
				status={card.face.status}
				body={card.face.body}
				transport={{
					badge: TRANSPORT_BADGE[TRANSPORT_OF[card.tile.marketplace]],
					line: transportLine(TRANSPORT_OF[card.tile.marketplace], card.tile.name)
				}}
				action={card.face.action}
			/>
		{/each}

		{#each PLANNED as tile (tile.slug)}
			<MarketplaceCard
				mark={tile.mark}
				name={tile.name}
				home={tile.home}
				status={{ tone: 'soon', label: 'Coming soon' }}
				body={tile.body}
				transport={tile.marketplace === undefined
					? undefined
					: {
							badge: TRANSPORT_BADGE[TRANSPORT_OF[tile.marketplace]],
							line: transportLine(TRANSPORT_OF[tile.marketplace], tile.name)
						}}
				pending
			/>
		{/each}

		{#each LISTED as tile (tile.slug)}
			<MarketplaceCard
				mark={tile.mark}
				name={tile.name}
				home={tile.home}
				status={{ tone: 'soon', label: 'On our list' }}
				body={tile.body}
				pending
			/>
		{/each}

		{#if requesting}
			<RequestCard onclose={() => (requesting = false)} />
		{:else}
			<AddCard
				title="Request a marketplace"
				why="Tell us where else you sell."
				onclick={() => (requesting = true)}
			/>
		{/if}
	</div>

	<section class="mp-part">
		<div class="mp-sect">
			<h2>Browser extension</h2>
			<p>
				An extension would let you connect from your browser instead of the desktop app.
			</p>
			<p>Neither is available yet, and the desktop app does everything an extension would.</p>
		</div>

		<div class="mp-grid">
			{#each EXTENSIONS as tile (tile.slug)}
				<MarketplaceCard
					mark={tile.mark}
					name={tile.name}
					status={{ tone: 'soon', label: 'Coming soon' }}
					body={tile.body}
					pending
				/>
			{/each}
		</div>
	</section>

	<div class="mp-part">
		<Downloads manifest={releases.data ?? null} />
	</div>

	<div class="mp-part">
		<Machines
			{devices}
			{joined}
			{now}
			pending={registry.isPending}
			failed={registry.isError}
		/>
	</div>

	<div class="mp-part">
		<Authorship rows={declarable} {now} {read} />
	</div>

	<p class="mp-disclaimer">{DISCLAIMER}</p>
</div>

{#snippet toCopyright()}
	<Button tier="outline" small href="#copyright">Declare it</Button>
{/snippet}
