<!-- Appearance, which is the whole of Preferences for now.

     Three choices and not a toggle, because System is a real answer: a seller
     whose machine turns dark at sunset has already made this decision once and
     should not have to make it again here.

     The choice lives in `localStorage` and nowhere else. Carrying it on the
     account, so a seller who signs in on a second machine finds the palette
     they picked, is phase 1's Capabilities work in
     `docs/notes/design/2026-09-12-one-marketplace-per-site-and-the-seller-workflows.md`;
     until then this says so on the panel rather than implying a sync that is
     not happening. -->
<script lang="ts">
	import Panel from '$lib/Panel.svelte';
	import { whereYouAre } from '$lib/machine-here';
	import { machineHere } from '$lib/machine.svelte';
	import { DARK_QUERY, commitThemeChoice, themeChoice, type ThemeChoice } from '$lib/theme';

	const OPTIONS: ReadonlyArray<{ id: ThemeChoice; label: string }> = [
		{ id: 'system', label: 'System' },
		{ id: 'light', label: 'Light' },
		{ id: 'dark', label: 'Dark' },
	];

	// Read once at construction from the same key `app.html` painted from, so
	// the control opens on the choice already on screen. `browser` is not
	// imported for this: the guard is the store itself, which a server render
	// does not have.
	const store = typeof localStorage === 'undefined' ? null : localStorage;
	let choice = $state<ThemeChoice>(themeChoice(store));

	function choose(next: ThemeChoice): void {
		choice = next;
		commitThemeChoice(
			store,
			document.documentElement,
			window.matchMedia(DARK_QUERY).matches,
			next,
		);
	}
</script>

<Panel
	title="Appearance"
	description="Which palette this console is drawn in, on this device."
>
	<!-- Which machine "this device" is, said in plain sight rather than left to
	     a tooltip. The console is one build served to a browser and to the app
	     window around it, and until this line nothing on any screen told the
	     two apart: the founder's 0.7.0 review asked for it on every platform,
	     and this is the screen a seller reaches when they want to know what
	     this copy of Teachouse is. The panel above it says a choice is kept on
	     this device, which is the sentence this one finishes. -->
	<p class="whereabouts">{whereYouAre(machineHere.where)}</p>

	<fieldset class="appearance">
		<legend>Theme</legend>
		<div class="seg" role="radiogroup" aria-label="Theme">
			{#each OPTIONS as option (option.id)}
				<button
					type="button"
					role="radio"
					aria-checked={choice === option.id}
					onclick={() => choose(option.id)}
				>
					{option.label}
				</button>
			{/each}
		</div>
	</fieldset>
</Panel>

<style>
	/* The machine line above the control, set as a statement rather than as a
	   label: it is not part of the theme choice and must not read as its
	   description. */
	.whereabouts {
		margin: 0 0 14px;
		color: var(--muted);
		font-size: 13px;
	}

	/* The segmented control is three buttons in one track, which is what the
	   console already draws for a tab bar; it is not `.tab-bar` because that
	   one is an underline on a rule and this one is a switch with a body. */
	.appearance {
		border: 0;
		padding: 0;
		margin: 0;
		min-inline-size: 0;
	}

	.appearance legend {
		font-size: 12.5px;
		font-weight: 600;
		color: var(--muted);
		padding: 0 0 7px;
	}

	.seg {
		display: inline-grid;
		grid-auto-flow: column;
		grid-auto-columns: 1fr;
		gap: 3px;
		padding: 3px;
		background: var(--hover);
		border: 1px solid var(--line);
		border-radius: var(--r-pill);
	}

	.seg button {
		min-height: var(--control-h-sm);
		padding: 0 18px;
		border: 0;
		border-radius: var(--r-pill);
		background: none;
		color: var(--muted);
		font: inherit;
		font-size: 13px;
		font-weight: 600;
		white-space: nowrap;
		cursor: pointer;
	}

	.seg button:hover {
		color: var(--text);
	}

	.seg button[aria-checked='true'] {
		background: var(--primary);
		color: var(--on-fill);
	}

	.seg button:focus-visible {
		outline: 2px solid var(--primary);
		outline-offset: 2px;
	}

	/* On a phone the three fill the width rather than sitting in a huddle at
	   the left edge, so each target is wide enough to hit. */
	@media (max-width: 620px) {
		.seg {
			display: grid;
		}

		.seg button {
			min-height: var(--control-h);
			padding: 0 10px;
		}
	}
</style>
