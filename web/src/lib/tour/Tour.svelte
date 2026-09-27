<script lang="ts">
	import { untrack } from 'svelte';
	import { goto } from '$app/navigation';
	import Button from '$lib/Button.svelte';
	import { placeTour, visibleRect, TOUR_STEPS, type Rect, type Size } from './model';
	import { tour } from './tour.svelte';

	/** Called once the tour closes, with how it ended. The shell records it on
	 *  the profile; the tour itself knows nothing of who is signed in. */
	let { onEnd }: { onEnd: (outcome: 'completed' | 'skipped') => void } = $props();

	let box = $state<HTMLDialogElement>();
	let primary = $state<HTMLElement>();
	let target = $state<Rect | null>(null);
	let viewport = $state<Size>({ width: 0, height: 0 });
	let cardHeight = $state(0);

	const step = $derived(TOUR_STEPS[tour.index]);
	const placement = $derived(
		placeTour(target, viewport, { width: viewport.width, height: cardHeight })
	);

	/** Measures the lit element and the window. The dialog is fixed over the
	 *  whole viewport, so the element's own client rectangle is already in
	 *  the dialog's coordinates. */
	function measure() {
		const size = { width: window.innerWidth, height: window.innerHeight };
		const anchor = step.anchor;
		viewport = size;
		target =
			anchor === null
				? null
				: visibleRect(
						Array.from(document.querySelectorAll(`[data-tour="${anchor}"]`), (element) =>
							element.getBoundingClientRect()
						),
						size
					);
	}

	// Opened as a modal dialog, which is what makes the page behind it inert
	// and puts it in the top layer; Escape arrives as `cancel` below.
	$effect(() => {
		if (box === undefined) {
			return;
		}
		if (tour.open && !box.open) {
			box.showModal();
		} else if (!tour.open && box.open) {
			box.close();
		}
	});

	// Re-measured on every step, and whenever the window moves under it.
	// Only the step is tracked: the measurement writes the state it would
	// otherwise read, and tracking that would re-run this without end.
	$effect(() => {
		if (!tour.open) {
			return;
		}
		void step;
		untrack(() => {
			measure();
			// The step's forward control takes the focus, so Enter walks on.
			primary?.querySelector<HTMLElement>('button')?.focus();
		});
		window.addEventListener('resize', measure);
		window.addEventListener('scroll', measure, true);
		return () => {
			window.removeEventListener('resize', measure);
			window.removeEventListener('scroll', measure, true);
		};
	});

	/** Closing on the last step is finishing it; anywhere earlier is a skip. */
	function end(finished: boolean) {
		const outcome = finished || tour.last ? 'completed' : 'skipped';
		tour.close();
		onEnd(outcome);
	}

	function keys(event: KeyboardEvent) {
		if (event.key === 'ArrowRight' && !tour.last) {
			event.preventDefault();
			tour.next();
		} else if (event.key === 'ArrowLeft') {
			event.preventDefault();
			tour.back();
		} else if (event.key === 'Tab' && box !== undefined) {
			// The modal dialog already makes the page inert, but lets Tab run
			// off its last control into the browser's own; this keeps it
			// cycling through the card's controls instead.
			const controls = Array.from(box.querySelectorAll<HTMLElement>('button, a[href]'));
			const first = controls[0];
			const last = controls.at(-1);
			if (first === undefined || last === undefined) {
				return;
			}
			if (event.shiftKey && document.activeElement === first) {
				event.preventDefault();
				last.focus();
			} else if (!event.shiftKey && document.activeElement === last) {
				event.preventDefault();
				first.focus();
			}
		}
	}

	async function createFirst() {
		end(true);
		await goto('/resources/new');
	}
</script>

<dialog
	bind:this={box}
	class="tour"
	aria-labelledby="tour-title"
	aria-describedby="tour-body"
	oncancel={(event) => {
		event.preventDefault();
		end(false);
	}}
	onkeydown={keys}
>
	{#if tour.open}
		{#if placement.spotlight}
			<div
				class="spot"
				style:top="{placement.spotlight.top}px"
				style:left="{placement.spotlight.left}px"
				style:width="{placement.spotlight.width}px"
				style:height="{placement.spotlight.height}px"
			></div>
		{:else}
			<div class="dim"></div>
		{/if}

		<div
			class="card"
			class:docked={placement.card.kind === 'docked'}
			class:on-edge={placement.card.kind === 'docked' && placement.card.bottom === 0}
			style:top={placement.card.kind === 'floating' ? `${placement.card.top}px` : null}
			style:left={placement.card.kind === 'floating' ? `${placement.card.left}px` : null}
			style:width={placement.card.kind === 'floating' ? `${placement.card.width}px` : null}
			style:bottom={placement.card.kind === 'docked' ? `${placement.card.bottom}px` : null}
			bind:clientHeight={cardHeight}
		>
			<p class="count">{tour.index + 1} of {TOUR_STEPS.length}</p>
			<h2 id="tour-title">{step.title}</h2>
			<p id="tour-body">{step.body}</p>
			<div class="actions">
				<button class="skip" type="button" onclick={() => end(false)}>
					{tour.last ? 'Close' : 'Skip tour'}
				</button>
				<span class="grow"></span>
				{#if tour.index > 0}
					<Button tier="quiet" onclick={() => tour.back()}>Back</Button>
				{/if}
				<span class="primary" bind:this={primary}>
					{#if tour.last}
						<Button tier="primary" icon="circle-plus" onclick={createFirst}>
							Create your first resource
						</Button>
					{:else}
						<Button tier="primary" onclick={() => tour.next()}>Next</Button>
					{/if}
				</span>
			</div>
		</div>
	{/if}
</dialog>

<style>
	/* The dialog is the whole viewport, transparent: the dimming is the
	   spotlight's own shadow, so the lit element shows through a hole
	   rather than being lifted above a scrim. */
	.tour {
		position: fixed;
		inset: 0;
		width: 100%;
		height: 100%;
		max-width: none;
		max-height: none;
		margin: 0;
		padding: 0;
		border: 0;
		background: transparent;
		overflow: hidden;
	}

	.tour::backdrop {
		background: transparent;
	}

	.spot {
		position: absolute;
		border-radius: var(--r-panel);
		box-shadow: 0 0 0 200vmax var(--scrim);
		outline: 2px solid var(--accent);
		pointer-events: none;
	}

	.dim {
		position: absolute;
		inset: 0;
		background: var(--scrim);
	}

	.card {
		position: absolute;
		display: grid;
		gap: var(--s-2);
		padding: var(--s-5);
		background: var(--surface);
		color: var(--text);
		border-radius: var(--r-card);
		box-shadow: var(--sh-3);
	}

	.card.docked {
		left: var(--s-2);
		right: var(--s-2);
	}

	/* On the bottom edge itself the card is a sheet: full width, square at
	   the foot, and clear of the phone's home indicator. */
	.card.on-edge {
		left: 0;
		right: 0;
		border-radius: var(--r-card) var(--r-card) 0 0;
		padding-bottom: calc(var(--s-5) + env(safe-area-inset-bottom));
	}

	.count {
		margin: 0;
		font-size: 12px;
		color: var(--muted);
	}

	h2 {
		margin: 0;
		font-family: var(--display);
		font-size: 18px;
	}

	#tour-body {
		margin: 0;
		line-height: 1.5;
	}

	.actions {
		display: flex;
		align-items: center;
		gap: var(--s-2);
		margin-top: var(--s-2);
		flex-wrap: wrap;
	}

	.grow {
		flex: 1;
	}

	.primary {
		display: contents;
	}

	.skip {
		border: 0;
		background: none;
		padding: 0;
		color: var(--muted);
		font: inherit;
		font-size: 14px;
		text-decoration: underline;
		cursor: pointer;
	}

	@media (prefers-reduced-motion: no-preference) {
		.spot,
		.card {
			transition:
				top 0.2s ease,
				left 0.2s ease,
				width 0.2s ease,
				height 0.2s ease,
				bottom 0.2s ease;
		}
	}
</style>
