<script lang="ts">
	import StatusPill, { type Tone } from '$lib/StatusPill.svelte';
	import { external } from '$lib/external';
	import { cardAnchor } from '$lib/platforms';
	import type { Marketplace } from '$lib/generated/vocab';
	import TileMark from './TileMark.svelte';
	import type { Mark } from './catalogue';
	import type { BusyAt, CardAction, HereFace } from './view';

	/** Whether the mark's own proportions are square, and so whether it takes
	 *  the narrow tile. A glyph is drawn square by definition; an image is
	 *  square only where its shape says so; a word never is. */
	const isSquare = (drawn: Mark) =>
		drawn.kind === 'glyph' || (drawn.kind === 'image' && drawn.shape === 'icon');

	let {
		mark,
		name,
		home,
		handle,
		status,
		body,
		here = null,
		about,
		transport,
		action,
		disconnect,
		signOut,
		running,
		refusal = null,
		onrun,
		ondisconnect,
		onsignout,
		pending = false
	}: {
		mark: Mark;
		name: string;
		/** The marketplace's own front page. The mark and the name both open it,
		 *  which is what makes each tile identify a marketplace rather than
		 *  merely name it, and what satisfies Shopify's condition that its
		 *  brand assets be shown with a link to its homepage, and Mozilla's that
		 *  its logo be shown in a visual referring or linking to the program.
		 *  Absent on the Chrome tile, which draws no vendor mark. */
		home?: string;
		/** The account the marketplace shows the seller, where it reported one.
		 *  An em dash where it did not, which is the specification's own
		 *  placeholder: the line is there on every connected card, so its
		 *  absence would move the status pill rather than say nothing. */
		handle?: string | null;
		status?: { tone: Tone; label: string };
		body: string;
		/** What the marketplace is, for a seller who does not recognise the name.
		 *  Shown rather than hidden behind a disclosure, because the reader it is
		 *  for is exactly the one who would not open a disclosure to find out.
		 *  Absent on the two browser tiles, which are not marketplaces. */
		about?: string;
		/** The transport class D1 requires on every marketplace row, as a badge
		 *  and as the sentence that says the same thing. Absent on a tile for a
		 *  marketplace the platform does not know, which has no recorded class
		 *  to state. */
		transport?: { badge: string; line: string };
		/** The card's one primary action, already carrying the marketplace's
		 *  name: a place to go, or an act this host can perform. The label is
		 *  rendered whole rather than composed with `name` here, because the
		 *  browser arm is a sentence and not a verb. */
		action?: CardAction;
		/** What this machine itself holds, where it answered. Stated beside the
		 *  organisation's own pill rather than in place of it, because they are
		 *  two facts: a marketplace can be connected for the account and absent
		 *  from the computer the seller is standing at, which is the case that
		 *  used to leave them no way in. Null where nothing may be claimed — a
		 *  browser, an application too old to answer, a read in flight. */
		here?: HereFace | null;
		/** The account-level way out, offered only where a connection stands.
		 *  Separate from `action` because a card can offer both, and because
		 *  this one is destructive and the other is not. */
		disconnect?: { label: string; marketplace: Marketplace };
		/** The local way out: this machine's own login, offered only where this
		 *  machine holds one. A third control rather than a mode of the second,
		 *  because the two reach different things — one machine's store, and the
		 *  organisation's connection — and a seller pressing either must know
		 *  which. */
		signOut?: { label: string; marketplace: Marketplace };
		/** Which control on this card is mid-flight. A connect opens the
		 *  marketplace's own sign-in — beside the console on a computer, in
		 *  place of it on a phone — and can stand for a minute either way, so a
		 *  card with no busy state reads as a button that did nothing. */
		running?: BusyAt;
		/** Why this card's action cannot run at all, where a plan withholds it
		 *  — the connection cap being the one that does. Null where it can.
		 *  Stated in `title` on the control, because a disabled control with no
		 *  reason reads as a fault; the route refuses the same request on its
		 *  own, so this is the earlier of two refusals rather than the only
		 *  one. */
		refusal?: string | null;
		onrun?: (marketplace: Marketplace) => void;
		ondisconnect?: (marketplace: Marketplace) => void;
		onsignout?: (marketplace: Marketplace) => void;
		/** Not built, and so not a state that can change: the card takes a
		 *  dashed edge and carries no action. */
		pending?: boolean;
	} = $props();
</script>

<!-- The id is derived here rather than passed in, because the card is handed
     the name and not the list entry it came from, and every caller would
     otherwise have to remember to pass an anchor for a link it does not itself
     write. `platforms.test.ts` holds the three marketplaces the console links
     into to the ids it links at. -->
<article id={cardAnchor(name)} class="mp-card" class:pending>
	<div class="mp-cap">
		<!-- The mark and the name lead to the same place, so only the name is a
		     tab stop and only the name is announced: a second link saying the
		     same thing is noise to anyone reading with a screen reader. Hiding a
		     focusable element from assistive technology is wrong, so where the mark
		     is a link `aria-hidden` and `tabindex="-1"` go together and neither
		     appears without the other. -->
		{#if home}
			<a
				class="mp-mark"
				class:mp-square={isSquare(mark)}
				href={home}
				target="_blank"
				rel="noopener noreferrer"
				aria-hidden="true"
				tabindex="-1"
				use:external
			>
				<TileMark {mark} />
			</a>
		{:else}
			<span class="mp-mark" class:mp-square={isSquare(mark)} aria-hidden="true">
				<TileMark {mark} />
			</span>
		{/if}
		<span class="mp-who">
			{#if home}
				<a class="t" href={home} target="_blank" rel="noopener noreferrer" use:external>
					{name}<span class="mp-away">(opens in a new tab)</span>
				</a>
			{:else}
				<span class="t">{name}</span>
			{/if}
			{#if handle !== undefined}<span class="handle">{handle ?? '—'}</span>{/if}
		</span>
		{#if status}
			<StatusPill tone={status.tone} label={status.label} />
		{/if}
	</div>

	<!-- What the marketplace is, before what is happening with it. Fifteen of
	     these cards carry the identical transport sentence in `body`, so leading
	     with that gives a seller who does not recognise the name the one line
	     that tells them nothing, and buries the line written for them under it. -->
	{#if about}
		<p class="mp-about">{about}</p>
	{/if}

	<p class="mp-body">{body}</p>

	{#if transport}
		<div class="mp-transport">
			<span>{transport.line}</span>
			<StatusPill tone="flat" label={transport.badge} />
		</div>
	{/if}

	<!-- What this machine holds, under what the account holds. Its own line
	     rather than a second pill in the header, because the header pill is the
	     organisation's and the two disagree exactly when it matters: a
	     marketplace connected on the seller's other computer, and absent from
	     this one. -->
	{#if here}
		<div class="mp-here">
			<span>{here.line}</span>
			<StatusPill tone={here.tone} label={here.label} />
		</div>
	{/if}

	{#if action || disconnect || signOut}
		<div class="mp-foot">
			{#if action}
				{#if action.kind === 'link'}
					<a class="go" href={action.href}>{action.label} <span aria-hidden="true">→</span></a>
				{:else}
					{@const marketplace = action.marketplace}
					<button
						class="go"
						type="button"
						disabled={running !== undefined || refusal !== null}
						title={refusal ?? undefined}
						onclick={() => onrun?.(marketplace)}
					>
						{running === 'action' ? `Signing in to ${name}…` : action.label}
						<span aria-hidden="true">→</span>
					</button>
				{/if}
			{/if}
			<!-- The two ways out, grouped so the foot stays one action on one side
			     and the ways out on the other however many of them a card has. The
			     local sign-out leads, because it is the smaller act of the two and
			     the one a seller on a shared machine wants. -->
			{#if signOut || disconnect}
				<span class="mp-offs">
					{#if signOut}
						{@const forgetting = signOut.marketplace}
						<button
							class="off"
							type="button"
							disabled={running !== undefined}
							onclick={() => onsignout?.(forgetting)}
						>
							{running === 'signout' ? 'Signing out…' : signOut.label}
						</button>
					{/if}
					{#if disconnect}
						{@const removing = disconnect.marketplace}
						<button
							class="off"
							type="button"
							disabled={running !== undefined}
							onclick={() => ondisconnect?.(removing)}
						>
							{running === 'disconnect' ? 'Disconnecting…' : disconnect.label}
						</button>
					{/if}
				</span>
			{/if}
		</div>
	{/if}
</article>

<style>
	/* A card arrived at by a link from somewhere else says so for a moment, and
	   clears the header band that would otherwise sit over it. Nothing moves
	   focus: the browser's own hash navigation scrolls, and a scripted focus
	   move would be behaviour with no way to test it here. */
	.mp-card {
		scroll-margin-top: 90px;
	}

	.mp-card:target {
		outline: 2px solid var(--accent);
		outline-offset: 3px;
	}
</style>
