<script lang="ts" module>
	export interface StepMark {
		/** The step's anchor id: the link lands on `#step-<id>`. */
		id: string;
		label: string;
		/** The step holds a complete answer. */
		done?: boolean;
	}
</script>

<script lang="ts">
	// The flow's map: every step by number and name, linked to its card, with
	// the finished ones ticked. A seller sees at a glance how far along they
	// are and where the next thing to do is.

	import Icon from '$lib/Icon.svelte';

	let { steps, label = 'Steps' }: { steps: readonly StepMark[]; label?: string } = $props();

	/** The first step not yet done: the one the seller is on. */
	const current = $derived(steps.find((step) => !step.done)?.id ?? null);
</script>

<nav class="stepper" aria-label={label}>
	<ol>
		{#each steps as step, index (step.id)}
			<li class:done={step.done} class:current={step.id === current}>
				<a href="#step-{step.id}" aria-current={step.id === current ? 'step' : undefined}>
					<span class="dot" aria-hidden="true">
						{#if step.done}<Icon name="check" size={13} />{:else}{index + 1}{/if}
					</span>
					<span class="word">{step.label}</span>
				</a>
			</li>
		{/each}
	</ol>
</nav>

<style>
	.stepper {
		margin: 0 0 var(--s-5);
		overflow-x: auto;
		scrollbar-width: none;
	}

	ol {
		display: flex;
		align-items: center;
		gap: 0;
		margin: 0;
		padding: 0;
		list-style: none;
		min-width: max-content;
	}

	li {
		display: flex;
		align-items: center;
	}

	/* The rule between two steps, drawn before every step but the first. */
	li + li::before {
		content: '';
		display: block;
		width: clamp(16px, 4vw, 48px);
		height: 2px;
		margin: 0 var(--s-2);
		border-radius: 1px;
		background: var(--line);
	}

	li.done + li::before {
		background: var(--accent);
	}

	a {
		display: inline-flex;
		align-items: center;
		gap: var(--s-2);
		padding: 4px 10px 4px 4px;
		border-radius: var(--r-pill);
		color: var(--muted);
		font-size: 13px;
		font-weight: 500;
		text-decoration: none;
		white-space: nowrap;
	}

	a:hover {
		background: var(--hover);
	}

	a:focus-visible {
		outline: 2px solid var(--accent);
		outline-offset: 2px;
	}

	.dot {
		display: inline-grid;
		place-items: center;
		width: 24px;
		height: 24px;
		border-radius: 50%;
		border: 1.5px solid var(--line);
		background: var(--surface);
		color: var(--muted);
		font-size: 12px;
		font-weight: 600;
	}

	.done .dot {
		border-color: var(--accent);
		background: var(--accent-soft);
		color: var(--ok-ink);
	}

	.current a {
		background: var(--additive-soft);
		color: var(--additive);
		font-weight: 600;
	}

	.current .dot {
		border-color: var(--additive);
		background: var(--additive);
		color: var(--on-fill);
	}

	/* A phone shows every number and only the current step's word: five
	   labels across 390px is a row nobody can read. */
	@media (max-width: 720px) {
		li:not(.current) .word {
			display: none;
		}

		li:not(.current) a {
			padding-right: 4px;
		}
	}
</style>
