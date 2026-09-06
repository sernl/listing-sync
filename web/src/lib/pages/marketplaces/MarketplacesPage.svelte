<script lang="ts">
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { api } from '$lib/api';
	import AddCard from '$lib/AddCard.svelte';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { currentSessionToken, listBrowserSessions } from '$lib/browser-sessions';
	import {
		APP_CANNOT_CONNECT,
		connectHere,
		desktopInvoker,
		forgetHere,
		type SessionOutcome
	} from '$lib/desktop';
	import { merge } from '$lib/device-merge';
	import { marketplaceRows, needingAttention } from '$lib/devices-view';
	import type { Marketplace } from '$lib/generated/vocab';
	import { TRANSPORT_OF } from '$lib/inventory';
	import { queryKeys } from '$lib/query';
	import { toast } from '$lib/toast';
	import Authorship from './Authorship.svelte';
	import Downloads from './Downloads.svelte';
	import Machines from './Machines.svelte';
	import MarketplaceCard from './MarketplaceCard.svelte';
	import RequestCard from './RequestCard.svelte';
	import { fetchManifest } from './api';
	import { DISCLAIMER, EXTENSIONS, LISTED, LIVE, MARK_ATTRIBUTION, PLANNED } from './catalogue';
	import {
		type Busy,
		CARD_NAME,
		CONNECT_RETURN_MARKETPLACE_PARAM,
		CONNECT_RETURN_PARAM,
		type DisconnectServer,
		type LiveRead,
		TRANSPORT_BADGE,
		busyAt,
		connectReturn,
		deviceBranchInTileOrder,
		disconnectAsk,
		disconnectLabel,
		disconnectSay,
		disconnectable,
		headerAction,
		hostOf,
		liveFace,
		signsInPlace,
		transportLine,
		withBusy
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

	// Read once as well: whether this console is running inside the application,
	// and on what, does not change while the page is open.
	const invoke = desktopInvoker();
	const userAgent = typeof navigator === 'undefined' ? null : navigator.userAgent;
	const host = hostOf(invoke);
	const header = headerAction(host);
	// Whether a sign-in started here replaces this page rather than opening a
	// window beside it, which is true on a phone alone. Two sentences depend on
	// it: what a connect says while the page is going away, and what a
	// disconnect admits it cannot remove.
	const inPlace = signsInPlace(invoke, userAgent);

	const queryClient = useQueryClient();

	let requesting = $state(false);

	const devices = $derived(registry.data?.devices ?? []);
	const connections = $derived(linked.data ?? []);
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
			return { tile, face: liveFace(held, host) };
		})
	);

	/** The stored connection for one marketplace, or undefined where the tenant
	 *  has none. `MarketplaceRow.connection` is null by construction on the
	 *  device branch, so the row cannot answer this and the list has to. */
	function connectionFor(marketplace: Marketplace) {
		return connections.find((entry) => entry.marketplace === marketplace);
	}

	/** Which cards are mid-flight, and at what. Per card, because the cards are
	 *  independent: two marketplaces are two logins in two windows, and the
	 *  application refuses a second window for the same marketplace itself. A
	 *  seller starting the second must not make the first read idle while its
	 *  own login window is still open. */
	let busy = $state<Busy>({});

	/** Ask this machine to open one marketplace's login.
	 *
	 *  Nothing is invalidated until it answers, and then both reads are: the
	 *  application checks in before it reports success, so by the time this
	 *  resolves the server has already lifted the connection to linked and the
	 *  refetched card is right without a poll. */
	const connecting = createMutation(() => ({
		mutationFn: (marketplace: Marketplace) => connectHere(invoke, marketplace),
		onSuccess: async (outcome: SessionOutcome, marketplace: Marketplace) => {
			const name = CARD_NAME[marketplace];
			if (outcome.kind === 'opening') {
				// Deliberately silent, and the card is left busy. The sign-in is
				// replacing this page within the quarter-second the application
				// waits before navigating, so a toast here would be shown to
				// nobody, and clearing the busy state would offer a button that
				// is about to disappear. What the seller is told arrives on the
				// way back, through `connectReturn` below.
				return;
			}
			if (outcome.kind === 'done') {
				toast('info', `${name} is connected on this machine.`);
			} else if (outcome.kind === 'unsupported') {
				toast('error', APP_CANNOT_CONNECT);
			} else if (outcome.kind === 'refused') {
				toast('error', outcome.detail);
			} else {
				// Unreachable from this page, which offers the command only where
				// there is an invoker. Said rather than swallowed, because a
				// silent button is the failure this whole change is fixing.
				toast('error', `${name} is connected from the Teachouse app on your computer.`);
			}
			await refetchConnections();
		},
		onError: () => {
			toast('error', 'The sign-in could not be opened on this machine.');
		},
		onSettled: (outcome: SessionOutcome | undefined, _error, marketplace: Marketplace) => {
			// Left busy on `opening`, which is the one outcome where the page
			// does not survive to show anything: a card that went idle would
			// offer its button again for the instant before the marketplace's
			// sign-in replaces it, and a seller who pressed it twice would have
			// asked for two navigations.
			if (outcome?.kind === 'opening') {
				return;
			}
			busy = withBusy(busy, marketplace, null);
		}
	}));

	/** Disconnect one marketplace: this machine first, then the control plane.
	 *
	 *  In that order because until the jar is gone the machine still holds
	 *  cookies for a marketplace the server has been told is disconnected, and
	 *  because its very next check-in would lift the row back to linked.
	 *
	 *  The server half runs whatever the device half answered. A machine that
	 *  could not forget is a reason to tell the seller so, never a reason to
	 *  leave scheduled work running. */
	const disconnecting = createMutation(() => ({
		mutationFn: async (marketplace: Marketplace) => {
			const forgotten: SessionOutcome =
				host === 'app'
					? await forgetHere(invoke, marketplace)
					: { kind: 'unavailable' };
			const connection = connectionFor(marketplace);
			// Caught rather than thrown, because the sentence depends on what
			// the device half already did and `onError` is not given it. A
			// server refusal after a machine has forgotten its login is the one
			// disconnect that half-happened, and it is `disconnectSay` that has
			// both facts to say so with.
			let server: DisconnectServer = { kind: 'moved', moved: 0 };
			if (connection !== undefined) {
				try {
					server = { kind: 'moved', moved: (await api.disconnect(connection.id)).connections };
				} catch {
					server = { kind: 'refused' };
				}
			}
			return { forgotten, server };
		},
		onSuccess: async (
			done: { forgotten: SessionOutcome; server: DisconnectServer },
			marketplace: Marketplace
		) => {
			const say = disconnectSay(marketplace, done.forgotten, done.server);
			toast(say.tone, say.message);
			await refetchConnections();
		},
		// The server half no longer throws, so this is left for a genuinely
		// unexpected one -- the device call itself failing outside its own four
		// answers.
		onError: () => {
			toast('error', 'The marketplace could not be disconnected.');
		},
		onSettled: (_data, _error, marketplace: Marketplace) => {
			busy = withBusy(busy, marketplace, null);
		}
	}));

	async function refetchConnections() {
		await Promise.all([
			queryClient.invalidateQueries({ queryKey: queryKeys.connections }),
			queryClient.invalidateQueries({ queryKey: queryKeys.devices })
		]);
	}

	/** Say what a sign-in that replaced this page ended up doing.
	 *
	 *  The other half of the phone's connect, and the only half a seller sees:
	 *  the page that pressed the button was unloaded by the navigation to the
	 *  marketplace, so the application returns here with the verdict in the
	 *  address rather than resolving a promise that no longer exists.
	 *
	 *  Cleared with a replace rather than left standing, because a reload would
	 *  otherwise repeat a sentence about a sign-in that finished minutes ago,
	 *  and because a verdict is not a place to go back to. The card itself is
	 *  not rendered from this: whether a marketplace is connected is read from
	 *  the server's connection list, so a parameter typed by hand changes one
	 *  line of copy and no state at all. */
	$effect(() => {
		const said = connectReturn(page.url.searchParams);
		if (said === null) {
			return;
		}
		toast(said.tone, said.message);
		const rest = new URLSearchParams(page.url.searchParams);
		rest.delete(CONNECT_RETURN_PARAM);
		rest.delete(CONNECT_RETURN_MARKETPLACE_PARAM);
		const search = rest.toString();
		void goto(search.length === 0 ? page.url.pathname : `${page.url.pathname}?${search}`, {
			replaceState: true,
			keepFocus: true,
			noScroll: true
		});
	});

	function connect(marketplace: Marketplace) {
		busy = withBusy(busy, marketplace, 'action');
		connecting.mutate(marketplace);
	}

	function disconnect(marketplace: Marketplace) {
		if (!confirm(disconnectAsk(marketplace, host, connectionFor(marketplace), inPlace))) {
			return;
		}
		busy = withBusy(busy, marketplace, 'disconnect');
		disconnecting.mutate(marketplace);
	}

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
			<Button
				tier={host === 'app' ? 'outline' : 'primary'}
				icon="circle-plus"
				href={header.href}>{header.label}</Button
			>
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
				about={card.tile.about}
				transport={{
					badge: TRANSPORT_BADGE[TRANSPORT_OF[card.tile.marketplace]],
					line: transportLine(TRANSPORT_OF[card.tile.marketplace], card.tile.name)
				}}
				action={card.face.action}
				disconnect={disconnectable(connectionFor(card.tile.marketplace))
					? {
							label: disconnectLabel(card.tile.marketplace),
							marketplace: card.tile.marketplace
						}
					: undefined}
				running={busyAt(busy, card.tile.marketplace)}
				onrun={connect}
				ondisconnect={disconnect}
			/>
		{/each}

		{#each PLANNED as tile (tile.slug)}
			<MarketplaceCard
				mark={tile.mark}
				name={tile.name}
				home={tile.home}
				status={{ tone: 'soon', label: 'Coming soon' }}
				body={tile.body}
				about={tile.about}
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
				about={tile.about}
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
					home={tile.home}
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
	<!-- The licence conditions themselves, each its own line rather than run into
	     the paragraph above: Google asks for its Creative Commons line "in the
	     creative", and the creative is the page drawing the robot. -->
	{#each MARK_ATTRIBUTION as sentence (sentence)}
		<p class="mp-disclaimer mp-attribution">{sentence}</p>
	{/each}
</div>

{#snippet toCopyright()}
	<Button tier="outline" small href="#copyright">Declare it</Button>
{/snippet}
