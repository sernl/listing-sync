<script lang="ts">
	import type { FormVocabularyView, NativeValueView } from '$lib/api';
	import Field from '$lib/Field.svelte';
	import FormSection from '$lib/FormSection.svelte';
	import type { Marketplace } from '$lib/generated/vocab';
	import { MARKETPLACE_WORD, MARK_SRC } from '$lib/platforms';
	import type { Refusal, TptDraft } from '$lib/tpt-form';

	// One marketplace's own panel, headed by its own mark and holding only
	// what that marketplace asks for. The founder's rule: a field that belongs
	// to one marketplace has to say so where it is asked.
	let {
		marketplace,
		draft,
		form,
		refusals = [],
		gatesLicence = false,
		licences = [],
		/** Whether this band asks rather than requires. A template's fields are
		 *  starting points, so the Required words are off there. */
		optional = false,
		set
	}: {
		marketplace: Marketplace;
		draft: TptDraft;
		form: FormVocabularyView | null;
		refusals?: readonly Refusal[];
		/** Whether this marketplace gates a rights grant under the pricing
		 *  branch the draft is on. */
		gatesLicence?: boolean;
		licences?: readonly NativeValueView[];
		optional?: boolean;
		set: <K extends keyof TptDraft>(field: K, value: TptDraft[K]) => void;
	} = $props();

	const anchor = $derived(marketplace === 'Tpt' ? 'tpt_options' : 'tes_options');
</script>

<FormSection group={anchor} mark={MARK_SRC[marketplace]} {refusals}>
	{#if marketplace === 'Tpt'}
		{#if form}
			<div class="res-picks" role="radiogroup" aria-label="Copyright">
				<p class="res-lede">{form.copyright.preamble}</p>
				{#each form.copyright.options as option (option.id)}
					<label class="res-pick">
						<input
							type="radio"
							name="copyright"
							checked={draft.copyright === option.id}
							onchange={() => set('copyright', option.id)}
						/>
						<span class="res-pick-t">{option.label}</span>
					</label>
				{/each}
			</div>

			{#if !draft.free}
				<Field
					label="Tax Code"
					id="draft-tax-code"
					required={!optional}
					hint="Needed so sales tax can be collected on TPT."
				>
					<select
						id="draft-tax-code"
						aria-required={optional ? undefined : 'true'}
						value={draft.taxCode ?? ''}
						onchange={(event) =>
							set('taxCode', event.currentTarget.value === '' ? null : event.currentTarget.value)}
					>
						<option value="">Select a tax code</option>
						{#each form.tax_codes as code (code.id)}
							<option value={code.id}>{code.label}</option>
						{/each}
					</select>
				</Field>
			{/if}

			<fieldset class="res-choices">
				<legend>Localization</legend>
				<label>
					<!-- Three states, because the sidecar holds three. A product that
					     was never asked draws the box indeterminate rather than
					     unticked: reading the two the same way is what would let a
					     first save turn "not stated" into "explicitly no". -->
					<input
						type="checkbox"
						checked={draft.appropriateForCountry === true}
						indeterminate={draft.appropriateForCountry === null}
						onchange={(event) => set('appropriateForCountry', event.currentTarget.checked)}
					/>
					{form.localisation.label ?? form.localisation.generic_label}
				</label>
			</fieldset>
		{:else}
			<p class="res-note">Loading TPT’s questions…</p>
		{/if}
	{/if}

	{#if gatesLicence}
		<Field
			label="Licence"
			id="draft-licence"
			required={!optional}
			hint="{MARKETPLACE_WORD[marketplace]} needs a licence to list this."
		>
			<select
				id="draft-licence"
				aria-required={optional ? undefined : 'true'}
				value={draft.licence ?? ''}
				onchange={(event) =>
					set('licence', event.currentTarget.value === '' ? null : event.currentTarget.value)}
			>
				<option value="">Choose a licence</option>
				{#each licences as option (option.id)}
					<option value={option.id}>{option.label}</option>
				{/each}
			</select>
		</Field>
	{/if}
</FormSection>
