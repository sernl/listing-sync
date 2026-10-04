<script lang="ts">
	// The two boxes every account ticks before it exists, and ticks again when
	// the terms change: the sign-up page and the console's re-consent sheet draw
	// the same words from `$lib/legal`, so what was agreed reads the same in
	// both places.

	import { external } from '$lib/external';
	import { AGE_BOX, PRIVACY_URL, TERMS_BOX, TERMS_URL } from '$lib/legal';

	let {
		terms = $bindable(false),
		age = $bindable(false),
		disabled = false,
		idPrefix = 'consent'
	}: {
		/** Box (a): the terms, the privacy policy and owning what they publish. */
		terms?: boolean;
		/** Box (b): 18 or older. */
		age?: boolean;
		disabled?: boolean;
		/** Prefixes the two inputs' ids, `<prefix>-terms` and `<prefix>-age`. */
		idPrefix?: string;
	} = $props();
</script>

<fieldset class="consent-boxes" id={`${idPrefix}-boxes`}>
	<legend class="sr-only">Your agreement</legend>
	<div class="consent-box">
		<input
			id={`${idPrefix}-terms`}
			name="consent-terms"
			type="checkbox"
			required
			{disabled}
			bind:checked={terms}
		/>
		<label for={`${idPrefix}-terms`}>
			{TERMS_BOX.lead}
			<a class="link" href={TERMS_URL} target="_blank" rel="noopener" use:external
				>{TERMS_BOX.terms}</a
			>
			{TERMS_BOX.join}
			<a class="link" href={PRIVACY_URL} target="_blank" rel="noopener" use:external
				>{TERMS_BOX.privacy}</a
			>{TERMS_BOX.tail}
		</label>
	</div>
	<div class="consent-box">
		<input
			id={`${idPrefix}-age`}
			name="consent-age"
			type="checkbox"
			required
			{disabled}
			bind:checked={age}
		/>
		<label for={`${idPrefix}-age`}>{AGE_BOX}</label>
	</div>
</fieldset>

<style>
	.consent-boxes {
		display: grid;
		gap: var(--s-2);
		margin: 0;
		padding: 0;
		border: 0;
		min-width: 0;
	}

	/* The whole row is the tap target: the label is a sibling of the box and
	   names it, so a press anywhere on the words ticks it, and the row is at
	   least the 44px a thumb needs. */
	.consent-box {
		display: grid;
		grid-template-columns: 20px 1fr;
		align-items: start;
		gap: var(--s-3);
		min-height: 44px;
		padding: var(--s-2) var(--s-3);
		border: 1px solid var(--line);
		border-radius: var(--r-field);
		background: var(--card);
	}

	.consent-box:has(input:checked) {
		border-color: var(--primary);
		background: var(--accent-soft);
	}

	.consent-box:has(input:focus-visible) {
		outline: 2px solid var(--accent);
		outline-offset: 2px;
	}

	.consent-box input {
		width: 20px;
		height: 20px;
		margin: 1px 0 0;
		accent-color: var(--primary);
		cursor: pointer;
	}

	.consent-box input:disabled {
		cursor: not-allowed;
	}

	.consent-box label {
		min-width: 0;
		font-size: 13.5px;
		line-height: 1.5;
		color: var(--text);
		cursor: pointer;
		overflow-wrap: anywhere;
	}
</style>
