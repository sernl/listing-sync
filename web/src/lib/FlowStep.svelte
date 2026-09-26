<script lang="ts">
	// One numbered step of a guided flow: a card with its number, a title, one
	// sentence saying what to do, and the controls. A closed step keeps its
	// place in the flow and says what it holds ("TPT → Tes · 155 resources"),
	// so the page reads top to bottom as a sentence the seller is completing.
	//
	// `action` is the step's own primary control, drawn at the card's top right
	// where the hand already is; `footer` sticks to the bottom of the card
	// while it scrolls, which is where a Preview or Save button belongs on a
	// long step.

	import type { Snippet } from 'svelte';
	import Icon from '$lib/Icon.svelte';

	let {
		n,
		id,
		title,
		hint,
		summary,
		open = $bindable(true),
		done = false,
		aside,
		action,
		actionInBar = false,
		footer,
		children
	}: {
		n: number;
		/** The anchor the stepper links to: `step-<id>`. */
		id: string;
		title: string;
		/** One sentence saying what to do. */
		hint?: string;
		/** What the step holds, shown while it is closed. */
		summary?: string;
		open?: boolean;
		/** Ticks the number: the step holds a complete answer. */
		done?: boolean;
		action?: Snippet;
		/** The page repeats `action` in its phone action bar, so the card
		 *  drops its own copy below 720px rather than drawing it twice. */
		actionInBar?: boolean;
		aside?: Snippet;
		footer?: Snippet;
		children: Snippet;
	} = $props();

	const base = $props.id();
</script>

<section class="flow-step" class:closed={!open} id="step-{id}" aria-labelledby="{base}-title">
	<header class="flow-step-head">
		<h2 class="flow-step-title" id="{base}-title">
			<button
				type="button"
				class="flow-step-toggle"
				aria-expanded={open}
				aria-controls="{base}-body"
				onclick={() => (open = !open)}
			>
				<span class="flow-num" class:done aria-hidden="true">
					{#if done}<Icon name="check" size={15} />{:else}{n}{/if}
				</span>
				<span class="sr-only">Step {n}:</span>
				<span class="flow-step-name">{title}</span>
				<span class="flow-chev" aria-hidden="true"><Icon name="chevron-down" size={16} /></span>
			</button>
		</h2>
		{#if aside}<div class="flow-step-aside">{@render aside()}</div>{/if}
		{#if action}<div class="flow-step-action" class:barred={actionInBar}>{@render action()}</div>{/if}
	</header>

	{#if !open}
		{#if summary}<p class="flow-summary">{summary}</p>{/if}
	{:else}
		<div class="flow-step-body" id="{base}-body">
			{#if hint}<p class="flow-hint">{hint}</p>{/if}
			{@render children()}
		</div>
		{#if footer}
			<footer class="flow-step-foot">{@render footer()}</footer>
		{/if}
	{/if}
</section>

<style>
	.flow-step {
		position: relative;
		background: var(--card);
		border: 1px solid var(--line);
		border-radius: var(--r-card);
		box-shadow: var(--sh-1);
		padding: var(--s-5);
		min-width: 0;
		scroll-margin-top: var(--s-5);
	}

	.flow-step.closed {
		padding-bottom: var(--s-4);
	}

	.flow-step-head {
		display: flex;
		align-items: center;
		gap: var(--s-3);
		flex-wrap: wrap;
	}

	.flow-step-title {
		margin: 0;
		flex: 1 1 auto;
		min-width: 0;
		font-size: inherit;
	}

	.flow-step-toggle {
		display: flex;
		align-items: center;
		gap: var(--s-3);
		width: 100%;
		padding: 0;
		border: 0;
		background: none;
		color: var(--text);
		font: inherit;
		text-align: left;
		cursor: pointer;
	}

	.flow-step-toggle:focus-visible {
		outline: 2px solid var(--accent);
		outline-offset: 4px;
		border-radius: var(--r-field);
	}

	.flow-num {
		flex: none;
		display: inline-grid;
		place-items: center;
		width: 32px;
		height: 32px;
		border-radius: 50%;
		background: var(--additive-soft);
		color: var(--additive);
		font-family: var(--display);
		font-size: 15px;
		font-weight: 600;
	}

	.flow-num.done {
		background: var(--accent-soft);
		color: var(--ok-ink);
	}

	.flow-step-name {
		flex: 1 1 auto;
		min-width: 0;
		font-family: var(--display);
		font-size: 19px;
		font-weight: 600;
		line-height: 1.25;
		letter-spacing: -0.01em;
	}

	.flow-chev {
		flex: none;
		display: inline-flex;
		color: var(--muted);
		transition: transform 0.15s ease;
	}

	.closed .flow-chev {
		transform: rotate(-90deg);
	}

	.flow-step-aside {
		display: flex;
		align-items: center;
		gap: var(--s-2);
	}

	.flow-step-action {
		display: flex;
		align-items: center;
		gap: var(--s-2);
		margin-left: auto;
	}

	.flow-hint {
		margin: 0 0 var(--s-4);
		color: var(--muted);
		font-size: 14px;
	}

	.flow-summary {
		margin: var(--s-2) 0 0 44px;
		color: var(--muted);
		font-size: 13.5px;
	}

	.flow-step-body {
		margin-top: var(--s-3);
		padding-left: 44px;
		display: flex;
		flex-direction: column;
		gap: var(--s-4);
		min-width: 0;
	}

	.flow-step-foot {
		position: sticky;
		bottom: var(--flow-bar-h, 0px);
		z-index: 2;
		display: flex;
		align-items: center;
		gap: var(--s-3);
		flex-wrap: wrap;
		margin: var(--s-5) calc(-1 * var(--s-5)) calc(-1 * var(--s-5));
		padding: var(--s-4) var(--s-5) var(--s-4) calc(var(--s-5) + 44px);
		background: color-mix(in srgb, var(--card) 94%, transparent);
		border-top: 1px solid var(--line);
		border-radius: 0 0 var(--r-card) var(--r-card);
		backdrop-filter: blur(6px);
	}

	@media (max-width: 720px) {
		.flow-step {
			padding: var(--s-4);
		}

		.flow-step-body {
			padding-left: 0;
		}

		.flow-summary {
			margin-left: 0;
		}

		.flow-step-name {
			font-size: 17px;
		}

		/* The step's own action moves to the page's sticky bar on a phone,
		   where the thumb is, when the page has put it there. */
		.flow-step-action.barred {
			display: none;
		}

		.flow-step-foot {
			margin: var(--s-4) calc(-1 * var(--s-4)) calc(-1 * var(--s-4));
			padding: var(--s-3) var(--s-4);
		}
	}
</style>
