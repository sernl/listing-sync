<script lang="ts">
	import StatusPill, { type Tone } from '$lib/StatusPill.svelte';
	import { external } from '$lib/external';
	import type { Marketplace } from '$lib/generated/vocab';
	import TileMark from './TileMark.svelte';
	import type { Mark } from './catalogue';
	import type { CardAction } from './view';

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
		about,
		transport,
		action,
		disconnect,
		running,
		onrun,
		ondisconnect,
		pending = false
	}: {
		mark: Mark;
		name: string;
		/** The marketplace's own front page. The mark and the name both open it,
		 *  which is what makes each tile identify a marketplace rather than
		 *  merely name it, and what satisfies Shopify's condition that its
		 *  brand assets be shown with a link to its homepage. Absent on the two
		 *  browser tiles, which are not marketplaces. */
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
		/** The quiet second control, offered only where there is a connection
		 *  to remove. Separate from `action` because a card can offer both, and
		 *  because this one is destructive and the other is not. */
		disconnect?: { label: string; marketplace: Marketplace };
		/** Which control on this card is mid-flight. A connect opens a login
		 *  window and can stand for a minute, so a card with no busy state
		 *  reads as a button that did nothing. */
		running?: 'action' | 'disconnect';
		onrun?: (marketplace: Marketplace) => void;
		ondisconnect?: (marketplace: Marketplace) => void;
		/** Not built, and so not a state that can change: the card takes a
		 *  dashed edge and carries no action. */
		pending?: boolean;
	} = $props();
</script>

<article class="mp-card" class:pending>
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

	{#if action || disconnect}
		<div class="mp-foot">
			{#if action}
				{#if action.kind === 'link'}
					<a class="go" href={action.href}>{action.label} <span aria-hidden="true">→</span></a>
				{:else}
					{@const marketplace = action.marketplace}
					<button
						class="go"
						type="button"
						disabled={running !== undefined}
						onclick={() => onrun?.(marketplace)}
					>
						{running === 'action' ? `Signing in to ${name}…` : action.label}
						<span aria-hidden="true">→</span>
					</button>
				{/if}
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
		</div>
	{/if}
</article>
