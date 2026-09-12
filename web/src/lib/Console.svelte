<script lang="ts">
	import type { Snippet } from 'svelte';
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { page } from '$app/state';
	import { goto, invalidateAll } from '$app/navigation';
	import { impersonationState, operatorVerdict } from '$lib/admin';
	import { api, avatarSrc } from '$lib/api';
	import { impersonatedSession, stopImpersonatingAndRestore } from '$lib/auth-client';
	import { present, type StatusPresentation } from '$lib/connection-status';
	import { checkInHere, desktopInvoker } from '$lib/desktop';
	import { signBackInRefusal, signedOutHere, SIGN_BACK_IN, whereYouAre } from '$lib/machine-here';
	import { machineHere } from '$lib/machine.svelte';
	import { renderFailureCause, renderFailureReport } from '$lib/render-failure';
	import { sectionAllowed, sectionReason } from '$lib/entitlement';
	import { entitlementRead, limitOf } from '$lib/entitlement-read';
	import Placeholder from '$lib/Placeholder.svelte';
	import Button from '$lib/Button.svelte';
	import { CARD_ANCHOR, MARKETPLACE_WORD } from '$lib/platforms';
	import Icon from '$lib/Icon.svelte';
	import ImpersonationBanner from '$lib/ImpersonationBanner.svelte';
	import {
		ADMIN_SECTION,
		CREATE_TAB,
		HOME_ITEM,
		PHONE_BAR,
		SECTIONS,
		breadcrumbFor,
		currentDestination,
		sectionFor
	} from '$lib/nav';
	import { accountTileState } from '$lib/account-tile.svelte';
	import { palette } from '$lib/palette.svelte';
	import { queryKeys } from '$lib/query';
	import SearchPalette from '$lib/SearchPalette.svelte';
	import { toast } from '$lib/toast';
	import { opensPalette } from '$lib/search-palette';

	let { children, onLogout }: { children: Snippet; onLogout: () => void } = $props();

	const organisation = createQuery(() => ({ queryKey: queryKeys.org, queryFn: () => api.org() }));
	const connections = createQuery(() => ({
		queryKey: queryKeys.connections,
		queryFn: () => api.connections()
	}));
	// The operator probe: one read of the cheapest operator route, cached for
	// the session. A 401 is the deliberate blank refusal every non-operator
	// gets, so it is not retried and never surfaces as a fault -- it is simply
	// how this client learns the human is not an operator.
	const probe = createQuery(() => ({
		queryKey: queryKeys.operator,
		queryFn: () => api.adminSyncHealth(),
		retry: false,
		staleTime: Infinity,
		gcTime: Infinity
	}));
	const verdict = $derived(
		operatorVerdict({ answered: probe.data !== undefined, failure: probe.error ?? null })
	);

	// The plan, beside the operator probe and for the same reason: one read
	// per session, shared by every page that draws a capped control. The rail
	// below hides a section this plan does not reach, which is courtesy —
	// every one of those routes refuses the request on its own.
	const entitlement = createQuery(() => entitlementRead);
	const caps = $derived(entitlement.data?.capabilities);

	// The identity session, read rather than remembered: a reload mid
	// impersonation must raise the banner from what it finds.
	const identity = createQuery(() => ({
		queryKey: queryKeys.identitySession,
		queryFn: () => impersonatedSession()
	}));
	const impersonation = $derived(impersonationState(identity.data ?? null));

	const queryClient = useQueryClient();

	// The console registers the machine it is running on, once per load, and
	// keeps what it answered.
	//
	// Here rather than on the machines page because this shell is what every
	// signed-in page renders inside, and a machine that only registered when
	// the seller happened to open Marketplaces would be missing from the list
	// the rest of the time. It is also the one place that knows a seller is
	// signed in: the layout renders this component only under a session, and a
	// session is exactly what the application's own start-up check-in lacks --
	// that runs before the sign-in, and then not again for an hour on a
	// computer or until the next resume on a phone.
	//
	// The answer is kept in `machineHere` rather than discarded, because it
	// carries the two facts every surface below now states: which machine this
	// is, and whether it was signed out from the console. The second is the
	// 0.7.0 defect -- a signed-out machine keeps its identity and wipes its
	// marketplace logins on every check-in, so the seller has to be told here,
	// on the machine it is happening to, rather than left to read a list.
	//
	// Once, not on every render: an `$effect` whose body reads nothing reactive
	// runs on mount alone. In a browser there is no invoker, the call answers
	// that nothing was reached, and nothing is refetched.
	$effect(() => {
		const invoke = desktopInvoker();
		void checkInHere(invoke).then((answer) => {
			machineHere.observe(answer, { inApp: invoke !== null });
			if (answer.reached) {
				// Only the device list, and only when a row may have appeared: the
				// machines page refreshes if it is open, and nothing else is
				// disturbed.
				void queryClient.invalidateQueries({ queryKey: queryKeys.devices });
			}
		});
	});

	// When this machine was signed out, for the banner's own sentence. Read
	// only once the check-in has said this machine is revoked, so an ordinary
	// session on an ordinary machine issues no list of the seller's machines
	// from the shell. The date comes out of the registry rather than the
	// heartbeat because the heartbeat answers a flag and the seller's question
	// is which day they did this.
	const registry = createQuery(() => ({
		queryKey: queryKeys.devices,
		queryFn: () => api.devices(),
		enabled: machineHere.revoked
	}));
	const signedOutAt = $derived(
		registry.data?.devices.find((device) => device.id === machineHere.where.device?.id)
			?.revoked_at ?? null
	);

	/** Signing this machine back in from wherever the seller is standing.
	 *
	 *  The same act the Machines list offers on this machine's own row, and
	 *  here as well because the banner is what a seller who has just signed in
	 *  actually meets: every page carries it, and asking them to find a list
	 *  first is asking them to walk past the sentence explaining why their
	 *  marketplace logins keep disappearing. */
	const signingBackIn = createMutation(() => ({
		mutationFn: () => machineHere.signBackIn(),
		onSuccess: async () => {
			toast('info', 'This machine is signed back in. Connect your marketplaces again on it.');
			await Promise.all([
				queryClient.invalidateQueries({ queryKey: queryKeys.devices }),
				queryClient.invalidateQueries({ queryKey: queryKeys.connections })
			]);
		},
		onError: (failure: unknown) => {
			toast('error', signBackInRefusal(failure));
		}
	}));

	let stopping = $state(false);
	let stopRefusal = $state<string | null>(null);

	async function stopImpersonating() {
		stopping = true;
		stopRefusal = null;
		try {
			await stopImpersonatingAndRestore();
			// Everything cached was read as the impersonated tenant. Clearing is
			// not tidiness: a stale organisation left in the cache would render
			// somebody else's workspace under the operator's own session.
			queryClient.clear();
			await invalidateAll();
			await goto('/admin/users');
		} catch (failure) {
			// The banner stays up rather than being dismissed, and the identity
			// session is deliberately not re-read: a failure here can leave the two
			// planes disagreeing, and a banner that vanished over that would hide
			// the one state that must stay visible. Logging out ends both.
			const said =
				failure instanceof Error ? failure.message : 'The impersonation was not stopped.';
			stopRefusal = `${said} You are still signed in as them — log out to end both sessions.`;
		} finally {
			stopping = false;
		}
	}

	/** Records a page that failed to draw, so a blank region leaves a trace.
	 *
	 *  `console.error` because this console has no diagnostics channel of its
	 *  own yet; when one exists, this is the single place that changes. */
	function reportDrawFailure(error: unknown) {
		console.error(renderFailureReport(error, page.url.pathname));
	}

	const pathname = $derived(page.url.pathname);
	const crumb = $derived(breadcrumbFor(pathname));
	const orgName = $derived(organisation.data?.name);
	// The seller's own picture, read here because this shell is the one
	// component that renders only under a session; the preferences screen
	// writes it and sets this same key.
	const profile = createQuery(() => ({
		queryKey: queryKeys.profile,
		queryFn: () => api.profile()
	}));
	const picture = $derived(avatarSrc(profile.data));
	// The strip's tile and the page header's avatar button are one derivation in
	// `account-tile.svelte.ts`, fed from here: this component holds both reads,
	// and a header that read them itself would fire them on the public pages
	// too.
	$effect(() => {
		accountTileState.observe({ picture, orgName });
	});
	const tile = $derived(accountTileState.tile);
	const links = $derived(connections.data ?? []);

	// Which section the rail lights and whose pages the card lists. The
	// operator's own section joins the rail only for a human the probe
	// admitted, and is asked for by name here so a non-operator on an `/admin`
	// path still lands on a section that exists rather than on a card of
	// destinations that answer 401.
	const operator = $derived(verdict === 'operator');
	const section = $derived(sectionFor(pathname, operator));
	// The destination this path belongs to, rather than a per-item `isCurrent`.
	// A page can be owned by an item it does not sit under — an import's detail
	// page lives at `/sync/requests/<id>` and belongs to Import — and only the
	// longest-claim answer gets that right without lighting two items elsewhere.
	const owning = $derived(currentDestination(pathname));
	// The card renders only for a section that has pages. The home path and the
	// open-questions queue belong to no section at all, and on those the rail
	// lights nothing rather than claiming one.
	const card = $derived(section !== null && section.items.length > 0 ? section : null);
	// Account sits at the foot of the rail beside Help, which is where the
	// founder's own sketch and Vendoo both put it; the rest run under the mark.
	//
	// A section this plan does not reach leaves the rail. Only once the plan
	// has been read: while the read is pending every section stands, because
	// a rail that filled in a second late would move under the seller's
	// cursor, and hiding a section they are entitled to is worse than showing
	// one they are not — the route refuses it either way.
	const railSections = $derived([
		...SECTIONS.filter(
			(entry) => entry.id !== 'account' && (caps === undefined || sectionAllowed(caps, entry.id))
		),
		...(operator ? [ADMIN_SECTION] : [])
	]);
	// Why the page in the region is not on this plan, where it is not. The
	// section rather than the path: a page inside a reachable section states
	// its own reason, because only it knows which capability it wanted.
	const gated = $derived(
		caps === undefined || section === null ? null : sectionReason(caps, section.id)
	);
	// The two controls in this shell that start a resource: the section card's
	// primary and the top strip's own. Crosslist is the only section that
	// declares a primary, and both land on the create form, so one figure
	// refuses both rather than two derivations that could disagree.
	const createRefusal = $derived(limitOf(entitlement.data, 'resources'));
	const accountSection = $derived(SECTIONS.find((entry) => entry.id === 'account'));

	/** The dot beside a marketplace in the top bar, for the two states the
	 *  strip raises at all. */
	const DOT: Record<StatusPresentation['tone'], string> = {
		ok: 'ok',
		mut: '',
		run: 'warn',
		bad: 'bad'
	};

	/** The marketplaces the strip has something to say about.
	 *
	 * `checking` and `connected` are removed entirely rather than greyed: a
	 * connection that is linked, or linked and verified, is nothing for the
	 * seller to act on, and a permanent row of reassurances is the thing they
	 * stop reading before the one row that matters appears (design of
	 * 2026-09-12, section 6). */
	const raised = $derived(
		links
			.map((link) => ({ link, shown: present(link.status) }))
			.filter((entry) => entry.shown.alert !== null)
	);

	// The palette is the console's search now: it floats over whichever page
	// the seller is on and opens the resource itself, rather than sending them
	// to the board filtered down to it. The board keeps its own search field
	// for narrowing what it is already showing. The flag is a module store
	// because the Resources page header opens it too, on a phone, where the
	// bar no longer has a search cell.
	function shortcut(event: KeyboardEvent) {
		if (opensPalette(event)) {
			event.preventDefault();
			palette.show();
		}
	}
</script>

<svelte:window onkeydown={shortcut} />

<svelte:head><title>{crumb} · Teachouse</title></svelte:head>

<div class="app" class:no-nav={card === null}>
	{#if impersonation}
		<ImpersonationBanner
			who={impersonation.who}
			{stopping}
			refusal={stopRefusal}
			onStop={stopImpersonating}
		/>
	{/if}

	<!-- This machine was signed out from the console, and is on every page
	     rather than on the Machines list alone: until it is signed back in the
	     app wipes its marketplace logins on every check-in, so a seller who
	     never opens Marketplaces would only see the consequence -- a Connect
	     that fails after they have typed a password -- and never the cause.
	     Raised only in the app: a browser is not a machine and can do nothing
	     about one. -->
	{#if machineHere.revoked}
		<div class="machine-out" role="alert">
			<span class="mark"><Icon name="laptop" size={14} /></span>
			<span class="said">{signedOutHere(signedOutAt)}</span>
			<button
				type="button"
				onclick={() => signingBackIn.mutate()}
				disabled={machineHere.restoring}
			>
				{machineHere.restoring ? 'Signing in…' : SIGN_BACK_IN}
			</button>
		</div>
	{/if}

	<nav class="rail" aria-label="Sections">
		<!-- The house alone, not the tiled mark: the rail's own ground is the
		     kit's indigo, and the mark is that same indigo with the house on it,
		     so the tile would vanish into the column it sits on. The tiled mark
		     is the favicon and the app icon, where there is no indigo behind
		     it. -->
		<a class="mark" href={HOME_ITEM.href} aria-label={HOME_ITEM.label}>
			<img src="/brand/house.svg" alt="Teachouse" width="36" height="36" />
		</a>

		{#each railSections as entry (entry.id)}
			<a
				class="rail-item"
				href={entry.href}
				aria-current={entry.id === section?.id ? 'page' : undefined}
				aria-label={entry.label}
				title={entry.hint}
			>
				<Icon name={entry.icon} size={20} />
			</a>
		{/each}

		<span class="rail-gap"></span>

		<a class="rail-item" href="/guides" aria-label="Help and guides" title="Help and guides">
			<Icon name="circle-question-mark" size={20} />
		</a>

		{#if accountSection}
			<a
				class="rail-item"
				href={accountSection.href}
				aria-current={accountSection.id === section?.id ? 'page' : undefined}
				aria-label={accountSection.label}
				title={accountSection.hint}
			>
				<Icon name={accountSection.icon} size={20} />
			</a>
		{/if}
	</nav>

	{#if card}
		<div class="nav-card">
			<div class="nav-top">
				<img class="nav-wordmark" src="/brand/wordmark.svg" alt="Teachouse" height="26" />
				<h2>{card.label}</h2>
				<p class="nav-hint">{card.hint}</p>
			</div>

			<!-- The section-primary control, refused at the catalogue cap rather
			     than opening a form the create route will then refuse. Plain
			     elements on the shell's own classes, as everything else in this
			     card is, and the reason travels in `title` exactly as
			     `Button.svelte` puts it there: a disabled control with no stated
			     reason reads as a fault. -->
			{#if card.primary}
				{#if createRefusal === null}
					<a class="cta nav-primary" href={card.primary.href}>
						<Icon name={card.primary.icon} size={16} />
						{card.primary.label}
					</a>
				{:else}
					<button class="cta nav-primary" type="button" disabled title={createRefusal}>
						<Icon name={card.primary.icon} size={16} />
						{card.primary.label}
					</button>
				{/if}
			{/if}

			<!-- Named directly rather than by `aria-labelledby`: the title it would
			     point at is hidden below the tablet breakpoint, and whether a hidden
			     element still supplies a name is not something to depend on. -->
			<nav aria-label={card.label}>
				{#each card.items as item (item.href)}
					<a
						class="nav-item"
						href={item.href}
						aria-current={item.href === owning?.href ? 'page' : undefined}
					>
						<span class="ico"><Icon name={item.icon} size={16} /></span>
						{item.label}
						{#if item.soon}
							<span class="chip">soon</span>
						{/if}
					</a>
				{/each}
			</nav>

			{#if card.id === 'account'}
				<button class="nav-item nav-foot" type="button" onclick={onLogout}>
					<span class="ico"><Icon name="log-out" size={16} /></span>
					Log out
				</button>
			{/if}
		</div>
	{/if}

	<main class="region">
		<div class="top">
			<span class="conn">
				{#each raised as { link, shown } (link.id)}
					<a
						class="conn-link"
						href={`/marketplaces#${CARD_ANCHOR[link.marketplace]}`}
						title={shown.explanation}
					>
						<span class="dot {DOT[shown.tone]}"></span>{MARKETPLACE_WORD[link.marketplace]}
						{shown.alert}
					</a>
				{/each}
			</span>
			<span class="grow"></span>
			<button class="search search-open" type="button" onclick={() => palette.show()}>
				<Icon name="search" size={15} />
				<span class="search-said">Search resources…</span>
				<kbd>ctrl K</kbd>
			</button>
			{#if createRefusal === null}
				<a class="cta" href={CREATE_TAB.href}>{CREATE_TAB.label}</a>
			{:else}
				<button class="cta" type="button" disabled title={createRefusal}>{CREATE_TAB.label}</button>
			{/if}
			<!-- Named here rather than by its contents: the avatar is `aria-hidden`
			     because the initials are decoration, so without this label the link
			     announces as the organisation's name rather than as Account. It
			     claims no `aria-current` -- the rail entry beside it already claims
			     the section, and this strip is not drawn on a phone at all.

			     The title says which machine the seller is at and whether they are
			     in the app, which is the founder's own ask: the console is one
			     build served to both hosts, so nothing on this strip distinguished
			     a browser tab from the app window around it. The Preferences head
			     carries the same sentence in plain sight; this is where somebody
			     already reaching for Account will find it. -->
			<a
				class="account"
				href="/settings"
				aria-label={accountSection?.label ?? 'Account'}
				title={whereYouAre(machineHere.where)}
			>
				<span class="avatar" aria-hidden="true">
					{#if tile.kind === 'picture'}
						<img
							class="avatar-pic"
							src={tile.src}
							alt=""
							onerror={() => accountTileState.unusable()}
						/>
					{:else if tile.kind === 'initials'}
						{tile.text}
					{/if}
				</span>
				<span class="who">
					<span class="org" title={orgName}>
						{#if orgName !== undefined}
							{orgName}
						{:else if organisation.isError}
							Organisation unavailable
						{:else}
							Loading…
						{/if}
					</span>
					<!-- The name the seller chose, in place of the host they are on:
					     the host told them nothing they decided. Rendered only once
					     the organisation has been read, because "no name yet" before
					     then is a claim about a row nobody has looked at. A newly
					     provisioned organisation never reaches this bar -- the claim
					     screen stands before it -- so the absent case here is only an
					     organisation that predates the slug, which the banner is
					     already asking about. -->
					<span class="plan">
						{#if organisation.data}{organisation.data.slug ?? 'No name yet'}{/if}
					</span>
				</span>
			</a>
		</div>

		<!-- A page that throws while drawing takes only its own region with it.
		     Without this the error escapes to the root and Svelte tears the whole
		     tree down, so the seller is left looking at nothing at all and has no
		     sentence to report; the shell around this boundary keeps drawing, so
		     the navigation still works and the failure is visibly local.

		     The failed arm is plain elements on the shell's own classes rather
		     than the components the rest of the console is built from. It renders
		     at the moment something has already gone wrong, and a component that
		     threw in here would escape to the root exactly as the page did. -->
		<!-- Keyed on the path so each route gets its own boundary. A boundary that
		     has tripped stays tripped until something recreates it, and this one
		     wraps every route, so before the key one page throwing left the
		     refusal standing over every page reached afterwards -- the seller saw
		     Marketplaces broken because Resources had been. The key costs no
		     remount that was not already happening: SvelteKit replaces the route
		     component on a path change regardless, and a query-only change, such
		     as the board's own filter, does not touch `pathname`. -->
		{#key pathname}
			<svelte:boundary onerror={(error) => reportDrawFailure(error)}>
				{#if gated !== null}
					<!-- The section is not on this plan. The page is not drawn at all
					     rather than drawn and then refused control by control: every
					     route inside it answers 422, so a rendered page would be a
					     screen of dead switches. -->
					<div class="page">
						<Placeholder icon="credit-card" headline="Not on your plan" body={gated}>
							{#snippet actions()}
								<Button tier="primary" href="/settings/subscription">See plans</Button>
							{/snippet}
						</Placeholder>
					</div>
				{:else}
					{@render children()}
				{/if}

				<!-- The error's own message under the sentence, in small muted type.
				     A seller who meets this on a phone has no other way to tell us
				     what happened: the console's log is behind a devtools pane they
				     cannot open, so the only line that can reach us is one they can
				     read off the screen. It is deliberately not styled as the
				     sentence above -- it is for us, and it says so by looking
				     like it. -->
				{#snippet failed(error)}
					<div class="page">
						<p>This page could not be drawn. Reload to try again.</p>
						<p class="drew-why">{renderFailureCause(error)}</p>
						<button class="btn" type="button" onclick={() => location.reload()}>Reload</button>
					</div>
				{/snippet}
			</svelte:boundary>
		{/key}
	</main>

	<nav class="tabbar" aria-label="Sections">
		<!-- Keyed by label rather than href: the create cell's href is the
		     Crosslist section's create route, not a destination the bar
		     navigates to, and keying by it would tie the key to a path that is
		     allowed to change. -->
		{#each PHONE_BAR as tab (tab.label)}
			{#if tab.create}
				<!-- The full label as the accessible name and the short word on
				     screen: "New" under a 48px disc is what fits, and "New
				     resource" is what the control does. No `aria-current` in any
				     state -- creating a resource is never the page you are on --
				     which is also why the disc carries no selected pill. -->
				<a class="tab-create" href={tab.href} aria-label={tab.label}>
					<span class="ring"><Icon name={tab.icon} size={24} /></span>
					<span>{tab.short ?? tab.label}</span>
				</a>
			{:else}
				<a
					class="tab-item"
					href={tab.href}
					aria-current={tab.href === section?.href ? 'page' : undefined}
				>
					<span class="ico"><Icon name={tab.icon} size={24} /></span>
					<span>{tab.short ?? tab.label}</span>
				</a>
			{/if}
		{/each}
	</nav>

	<SearchPalette bind:open={palette.open} />
</div>

<style>
	/* The wordmark above the section name, and the section's one sentence
	   beneath it. Only what these two elements need: `shell.css` owns the card
	   and stacks `.nav-top`, including hiding the whole header at the tablet
	   breakpoint, so neither rule here touches that block. */
	.nav-wordmark {
		display: block;
		width: auto;
		height: 26px;
	}

	.nav-hint {
		margin: 0;
		color: var(--muted);
		font-size: 12px;
		line-height: 1.4;
	}

	/* The strip's one raised marketplace. `shell.css` owns `.conn` and the
	   dot; this is only what makes the row a destination, because a notice
	   the seller cannot act on from where they read it is a dead end. */
	.conn-link {
		display: inline-flex;
		align-items: center;
		gap: 5px;
		white-space: nowrap;
		color: inherit;
		text-decoration: none;
	}

	.conn-link:hover {
		text-decoration: underline;
	}

	/* The signed-out machine's banner. The impersonation banner's own shape --
	   a full-width strip at the top of the grid -- because it is the same kind
	   of statement: a condition the whole console is being read under, which
	   no page can restate for itself. Its colour is the warning rather than
	   the danger one: nothing is broken and no data is at risk, but every
	   marketplace login on this machine is being wiped until the seller acts.
	   Local rather than in `app.css` because this shell is the only place it
	   is drawn. */
	.machine-out {
		grid-column: 1 / -1;
		position: sticky;
		top: 0;
		z-index: 19;
		display: flex;
		align-items: center;
		gap: 12px;
		flex-wrap: wrap;
		padding: 10px 20px;
		background: var(--warn-soft);
		color: var(--text);
		border-bottom: 1px solid var(--warn);
		font-size: 13px;
	}

	.machine-out .mark {
		display: inline-flex;
		align-items: center;
		/* The ink pair the contrast test measures on the soft ground, not the
		   raw accent, which is a fill and a line colour. */
		color: var(--warn-ink);
	}

	.machine-out .said {
		min-width: 0;
	}

	.machine-out button {
		margin-left: auto;
		min-height: var(--control-h-sm);
		padding: 5px 14px;
		border: 1px solid var(--warn);
		border-radius: var(--r-pill);
		background: var(--card);
		color: var(--text);
		font: inherit;
		font-size: 12.5px;
		font-weight: 600;
		cursor: pointer;
	}

	.machine-out button:disabled {
		opacity: 0.65;
		cursor: default;
	}

	/* Below the phone breakpoint the shell's grid is two columns and the first
	   row is the header band, which is where the impersonation banner puts
	   itself too. */
	@media (max-width: 620px) {
		.machine-out {
			grid-column: 1 / span 2;
			align-self: start;
		}
	}

	/* What the page threw, for a seller to read back to us. Muted and small,
	   because it is a diagnostic rather than an instruction, and `break-word`
	   because the one thing it must not do is push a phone's layout sideways
	   on a long message. */
	.drew-why {
		margin: 0 0 12px;
		color: var(--muted);
		font-size: 12px;
		line-height: 1.5;
		overflow-wrap: break-word;
	}
</style>
