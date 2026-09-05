<script lang="ts">
	import StatusPill, { type Tone } from '$lib/StatusPill.svelte';
	import { type Mark, wordmarkSize } from './catalogue';

	let {
		mark,
		name,
		home,
		handle,
		status,
		body,
		transport,
		action,
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
		/** The transport class D1 requires on every marketplace row, as a badge
		 *  and as the sentence that says the same thing. Absent on a tile for a
		 *  marketplace the platform does not know, which has no recorded class
		 *  to state. */
		transport?: { badge: string; line: string };
		action?: { label: string; href: string };
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
		     focusable element from assistive technology is wrong, so the pair
		     `aria-hidden` and `tabindex="-1"` go together and neither appears
		     without the other. -->
		{#if home}
			<a
				class="mp-mark"
				href={home}
				target="_blank"
				rel="noopener noreferrer"
				aria-hidden="true"
				tabindex="-1"
			>
				{#if mark.kind === 'image'}
					<img src={mark.src} alt="" loading="lazy" />
				{:else}
					<span class="word" style="font-size: {wordmarkSize(mark.text)}px">{mark.text}</span>
				{/if}
			</a>
		{:else}
			<span class="mp-mark">
				{#if mark.kind === 'image'}
					<img src={mark.src} alt="" loading="lazy" />
				{:else}
					<span class="word" style="font-size: {wordmarkSize(mark.text)}px">{mark.text}</span>
				{/if}
			</span>
		{/if}
		<span class="mp-who">
			{#if home}
				<a class="t" href={home} target="_blank" rel="noopener noreferrer">
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

	<p class="mp-body">{body}</p>

	{#if transport}
		<div class="mp-transport">
			<span>{transport.line}</span>
			<StatusPill tone="flat" label={transport.badge} />
		</div>
	{/if}

	{#if action}
		<div class="mp-foot">
			<a class="go" href={action.href}>{action.label} {name} <span aria-hidden="true">→</span></a>
		</div>
	{/if}
</article>
