<script lang="ts">
	import Button from '$lib/Button.svelte';
	import Explain from '$lib/Explain.svelte';
	import Icon from '$lib/Icon.svelte';
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
		 *  Behind the tile's (i) since 0.13: the tile says what to do, and the
		 *  description is one press away for the seller who needs it. */
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
     into to the ids it links at.

     One large tile per shop: the mark, the state, the machine it runs on and
     the one thing to press. What the marketplace is sits behind the (i), for
     the seller who does not recognise the name. -->
<article id={cardAnchor(name)} class="mp-tile" class:pending>
	<div class="mp-cap">
		<!-- The mark and the name lead to the same place, so only the name is a
		     tab stop and only the name is announced. Where the mark is a link
		     `aria-hidden` and `tabindex="-1"` go together. -->
		{#if home}
			<a
				class="mp-mark mp-big"
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
			<span class="mp-mark mp-big" class:mp-square={isSquare(mark)} aria-hidden="true">
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
		{#if about}
			<Explain title="What {name} is" label="">
				<p>{about}</p>
				{#if transport}<p>{transport.line}</p>{/if}
			</Explain>
		{/if}
	</div>

	{#if status}
		<div class="mp-state">
			<StatusPill tone={status.tone} label={status.label} />
			{#if transport}<StatusPill tone="flat" label={transport.badge} />{/if}
		</div>
	{/if}

	<!-- The machine the marketplace runs on, which is what the sign-in line
	     names. -->
	<p class="mp-device">
		<Icon name="laptop" size={16} />
		<span>{body}</span>
	</p>

	<!-- What this machine holds, beside what the account holds: the two disagree
	     exactly when it matters, a marketplace connected on the seller's other
	     computer and absent from this one. -->
	{#if here}
		<p class="mp-here">
			<StatusPill tone={here.tone} label={here.label} />
			<span>{here.line}</span>
		</p>
	{/if}

	{#if action || disconnect || signOut}
		<div class="mp-foot">
			{#if action}
				{#if action.kind === 'link'}
					<Button tier="primary" href={action.href}>{action.label}</Button>
				{:else}
					{@const marketplace = action.marketplace}
					<Button
						tier="primary"
						disabled={running !== undefined || refusal !== null}
						reason={refusal ?? (running !== undefined ? 'Working on it…' : undefined)}
						onclick={() => onrun?.(marketplace)}
					>
						{running === 'action' ? `Signing in to ${name}…` : action.label}
					</Button>
				{/if}
			{/if}
			<!-- The two ways out, the smaller act first: signing this machine out
			     touches this machine alone; disconnecting stops work everywhere. -->
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
	   clears the header band that would otherwise sit over it. */
	.mp-tile {
		scroll-margin-top: 90px;
	}

	.mp-tile:target {
		outline: 2px solid var(--accent);
		outline-offset: 3px;
	}
</style>
