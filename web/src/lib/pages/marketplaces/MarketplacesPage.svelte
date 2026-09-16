<script lang="ts">
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { api } from '$lib/api';
	import AddCard from '$lib/AddCard.svelte';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { entitlementRead, limitOf } from '$lib/entitlement-read';
	import {
		APP_CANNOT_CONNECT,
		connectHere,
		desktopInvoker,
		forgetHere,
		type LocalSessionOutcome,
		sessionStatusHere,
		type SessionOutcome
	} from '$lib/desktop';
	import { marketplaceRows, needingAttention } from '$lib/devices-view';
	import type { Marketplace } from '$lib/generated/vocab';
	import { TRANSPORT_OF } from '$lib/inventory';
	import { queryKeys } from '$lib/query';
	import { toast } from '$lib/toast';
	import Authorship from './Authorship.svelte';
	import ConsentDialog from './ConsentDialog.svelte';
	import { blockedBanner, needsConsent, standingFor } from './consent';
	import Downloads from './Downloads.svelte';
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
		MACHINES_ANCHOR,
		TRANSPORT_BADGE,
		busyAt,
		connectReturn,
		connectConsentRequired,
		connectSignedOut,
		deviceBranchInTileOrder,
		disconnectAsk,
		disconnectLabel,
		disconnectSay,
		disconnectable,
		headerAction,
		heldHere,
		hereFace,
		hostOf,
		liveFace,
		signOutHereAsk,
		signOutHereLabel,
		signOutHereSay,
		signsInPlace,
		transportLine,
		withBusy
	} from './view';
	import './marketplaces.css';

	$effect(() => {
		if (page.url.hash === '#machines') {
			void goto(`${MACHINES_ANCHOR.split('#')[0]}${page.url.search}#machines`, {
				replaceState: true
			});
		}
	});

	const registry = createQuery(() => ({ queryKey: queryKeys.devices, queryFn: () => api.devices() }));
	const linked = createQuery(() => ({
		queryKey: queryKeys.connections,
		queryFn: () => api.connections()
	}));
	const releases = createQuery(() => ({
		queryKey: queryKeys.downloadsManifest,
		queryFn: () => fetchManifest()
	}));
	// The seller-device consent record. Read here rather than off the device,
	// because the grant is per organisation and the card's Connect follows it:
	// a marketplace with no standing grant opens the notice instead.
	const consentRead = createQuery(() => ({
		queryKey: queryKeys.consents,
		queryFn: () => api.consents()
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

	// The plan, from the cache the shell filled. Only an unlinked card is
	// refused: re-signing in to a marketplace this tenant already holds adds
	// no connection, and disabling that control at the cap would strand a
	// seller whose one link had expired.
	const plan = createQuery(() => entitlementRead);
	const connectRefusal = $derived(limitOf(plan.data, 'marketplaces'));

	let requesting = $state(false);

	const devices = $derived(registry.data?.devices ?? []);
	const connections = $derived(linked.data ?? []);
	const rows = $derived(marketplaceRows(devices, connections, now));

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

	/** What THIS machine holds, one answer per live marketplace, as this
	 *  machine itself answered it.
	 *
	 *  A separate read from everything above, because it is a separate fact: the
	 *  connection list says some machine of the seller's is signed in, and the
	 *  card's Connect has to follow what the machine in front of them holds. An
	 *  unanswered marketplace is absent from the map and is handed on as null,
	 *  never as "not connected".
	 *
	 *  Fenced by a generation counter, as the import screen's own local read is:
	 *  a visibility refresh while a slower read is in flight would otherwise
	 *  land the older answer last. */
	let localRead = 0;
	let localSessions = $state<Map<Marketplace, LocalSessionOutcome>>(new Map());

	async function loadLocalSessions() {
		if (invoke === null) {
			return;
		}
		const current = ++localRead;
		const answers = await Promise.all(
			LIVE.map(async (tile) => ({
				marketplace: tile.marketplace,
				outcome: await sessionStatusHere(invoke, tile.marketplace)
			}))
		);
		if (current !== localRead) {
			return;
		}
		localSessions = new Map(answers.map((answer) => [answer.marketplace, answer.outcome]));
	}

	/** Read on entry, and again whenever this window comes back to the seller.
	 *
	 *  A marketplace login is captured in a window beside this page, or on
	 *  another machine entirely, so the answer goes stale while the console is
	 *  in the background and nothing tells it. The teardown advances the fence
	 *  so a read in flight when the page goes cannot write into the next one. */
	$effect(() => {
		const refresh = () => void loadLocalSessions();
		const visible = () => {
			if (document.visibilityState === 'visible') {
				refresh();
			}
		};
		refresh();
		window.addEventListener('focus', refresh);
		document.addEventListener('visibilitychange', visible);
		return () => {
			localRead += 1;
			window.removeEventListener('focus', refresh);
			document.removeEventListener('visibilitychange', visible);
		};
	});

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
			const local = localSessions.get(tile.marketplace) ?? null;
			const held: LiveRead =
				read === 'read' && row !== undefined ? { state: 'read', row } : { state: read === 'failed' ? 'failed' : 'pending' };
			return {
				tile,
				face: liveFace(held, host, local),
				here: hereFace(local, tile.marketplace),
				heldHere: heldHere(local)
			};
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
			} else if (outcome.kind === 'signedOut') {
				// The application refused before opening anything, because this
				// machine was signed out from the console. The same sentence the
				// phone's return leg carries, so the two surfaces say one thing
				// about one state.
				toast('error', connectSignedOut(marketplace).message);
			} else if (outcome.kind === 'unsupported') {
				toast('error', APP_CANNOT_CONNECT);
			} else if (outcome.kind === 'refused') {
				toast('error', outcome.detail);
			} else if (outcome.kind === 'consentRequired') {
				// The application read the record itself and refused in front of
				// the password: the grant was withdrawn since this page read it.
				toast('error', connectConsentRequired(marketplace).message);
			} else {
				// Unreachable from this page, which offers the command only where
				// there is an invoker. Said rather than swallowed, because a
				// silent button is the failure this whole change is fixing.
				toast('error', `${name} is connected from the Teachouse app on your computer.`);
			}
			await Promise.all([loadLocalSessions(), refetchConnections()]);
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

	/** Sign this machine out of one marketplace, and nothing else.
	 *
	 *  `forgetHere` alone, deliberately: the account-level disconnect is the
	 *  other control on the card and is the only one that asks the control
	 *  plane. One control that did both is the defect this pair replaces — a
	 *  seller signing a shared computer out of TPT stopped scheduled work on
	 *  every other machine they own.
	 *
	 *  The local read is taken again afterwards, because it is the fact the
	 *  card's Connect now follows. The server reads are invalidated too: the
	 *  connection's standing follows what machines report, and the application
	 *  may already have checked in by the time this resolves. */
	const signingOut = createMutation(() => ({
		mutationFn: (marketplace: Marketplace) => forgetHere(invoke, marketplace),
		onSuccess: async (forgotten: SessionOutcome, marketplace: Marketplace) => {
			const say = signOutHereSay(marketplace, forgotten);
			toast(say.tone, say.message);
			await Promise.all([loadLocalSessions(), refetchConnections()]);
		},
		onError: () => {
			toast('error', 'The login could not be removed from this machine.');
		},
		onSettled: (_data, _error, marketplace: Marketplace) => {
			busy = withBusy(busy, marketplace, null);
		}
	}));

	/** Disconnect one marketplace from the account: the control plane alone.
	 *
	 *  No machine's login is touched here, and the prompt says as much rather
	 *  than implying otherwise — a machine that keeps checking in while holding
	 *  the login lifts the connection back to linked on its next beat, which is
	 *  exactly what a check-in is for. Removing a login is the other control,
	 *  on the machine that holds it. */
	const disconnecting = createMutation(() => ({
		mutationFn: async (marketplace: Marketplace): Promise<DisconnectServer> => {
			const connection = connectionFor(marketplace);
			if (connection === undefined) {
				return { kind: 'moved', moved: 0 };
			}
			// Caught rather than thrown, because a refusal is one of the two
			// answers `disconnectSay` words and `onError` is handed none of
			// them.
			try {
				return { kind: 'moved', moved: (await api.disconnect(connection.id)).connections };
			} catch {
				return { kind: 'refused' };
			}
		},
		onSuccess: async (server: DisconnectServer, marketplace: Marketplace) => {
			const say = disconnectSay(marketplace, server);
			toast(say.tone, say.message);
			// The local read as well, though this act reaches no machine: the
			// card's Connect follows it, and the seller is looking at the card
			// the toast is about.
			await Promise.all([loadLocalSessions(), refetchConnections()]);
		},
		// The one call it makes catches its own refusal, so this is left for a
		// genuinely unexpected failure.
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
		void loadLocalSessions();
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

	/** The marketplace whose notice is open, or null. */
	let consentFor = $state<Marketplace | null>(null);

	function connect(marketplace: Marketplace) {
		// The notice first, where no grant stands. The card's action is
		// withheld while the read is pending, so this branch never runs on an
		// unknown answer.
		if (needsConsent(marketplace) && standingFor(consentRead.data, marketplace) !== 'granted') {
			consentFor = marketplace;
			return;
		}
		busy = withBusy(busy, marketplace, 'action');
		connecting.mutate(marketplace);
	}

	/** The grant landed: the marketplace the notice was for is now connected
	 *  the ordinary way, without the seller pressing the card again. */
	function agreed() {
		const marketplace = consentFor;
		consentFor = null;
		if (marketplace === null) {
			return;
		}
		busy = withBusy(busy, marketplace, 'action');
		connecting.mutate(marketplace);
	}

	/** Why one card's Connect is withheld on the consent read alone, or null. */
	function consentRefusal(marketplace: Marketplace): string | null {
		if (!needsConsent(marketplace)) {
			return null;
		}
		return standingFor(consentRead.data, marketplace) === 'unknown'
			? 'Checking your permissions…'
			: null;
	}

	/** The linked seller-device marketplace whose work has stopped for want of
	 *  a standing grant, or null. */
	const blocked = $derived(read === 'read' ? blockedBanner(rows, consentRead.data) : null);

	function disconnect(marketplace: Marketplace) {
		if (!confirm(disconnectAsk(marketplace, connectionFor(marketplace)))) {
			return;
		}
		busy = withBusy(busy, marketplace, 'disconnect');
		disconnecting.mutate(marketplace);
	}

	function signOutOfHere(marketplace: Marketplace) {
		if (!confirm(signOutHereAsk(marketplace, inPlace))) {
			return;
		}
		busy = withBusy(busy, marketplace, 'signout');
		signingOut.mutate(marketplace);
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
				Connect the places you sell. Your marketplace logins stay on your own computer.
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
		<Banner tone="warn" title="TPT needs to know who holds the copyright" action={toCopyright}>
			Tell us who holds the copyright for your work. Until you do, nothing you send to TPT
			will go live.
		</Banner>
	{/if}

	{#if blocked !== null}
		<Banner tone="warn" title={blocked.title} action={toPermissions}>
			Nothing new is started on {CARD_NAME[blocked.marketplace]} on any of your machines until
			you grant it. Logins already on your machines are untouched.
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
				here={card.here}
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
				signOut={card.heldHere
					? {
							label: signOutHereLabel(card.tile.marketplace),
							marketplace: card.tile.marketplace
						}
					: undefined}
				running={busyAt(busy, card.tile.marketplace)}
				refusal={connectionFor(card.tile.marketplace) === undefined
					? (connectRefusal ?? consentRefusal(card.tile.marketplace))
					: consentRefusal(card.tile.marketplace)}
				onrun={connect}
				ondisconnect={disconnect}
				onsignout={signOutOfHere}
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

{#if consentFor !== null}
	<ConsentDialog
		open={consentFor !== null}
		marketplace={consentFor}
		onAgreed={agreed}
		onClose={() => (consentFor = null)}
	/>
{/if}

{#snippet toCopyright()}
	<Button tier="outline" small href="#copyright">Declare it</Button>
{/snippet}

{#snippet toPermissions()}
	<Button tier="outline" small href="/settings#permissions">Grant it</Button>
{/snippet}
