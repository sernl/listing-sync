<script lang="ts">
	import type { FormVocabularyView } from '$lib/api';
	import Field from '$lib/Field.svelte';
	import FormSection from '$lib/FormSection.svelte';
	import { GROUP_HELP, type Advisory, type Refusal, type TptDraft } from '$lib/tpt-form';

	let {
		draft,
		form,
		refusals = [],
		advisories = [],
		/** Whether this band asks rather than requires. A template's fields are
		 *  starting points, so the Required words and the `aria-required` that
		 *  announces them are off there. */
		optional = false,
		set
	}: {
		draft: TptDraft;
		form: FormVocabularyView | null;
		refusals?: readonly Refusal[];
		advisories?: readonly Advisory[];
		optional?: boolean;
		set: <K extends keyof TptDraft>(field: K, value: TptDraft[K]) => void;
	} = $props();
</script>

<FormSection group="price" icon="credit-card" help={GROUP_HELP.price} {refusals} {advisories}>
	<fieldset class="res-choices">
		<legend>Free or paid</legend>
		<label>
			<input
				type="checkbox"
				checked={draft.free}
				onchange={(event) => set('free', event.currentTarget.checked)}
			/>
			Free Resource
		</label>
		{#if form}
			<span class="res-note">
				Keep a free resource to {form.limits.free_resource_page_guidance} pages or fewer.
			</span>
		{/if}
	</fieldset>

	{#if !draft.free}
		<div class="res-row">
			<Field label="Price" id="draft-price" required={!optional}>
				<input
					id="draft-price"
					type="text"
					inputmode="decimal"
					aria-required={optional ? undefined : 'true'}
					placeholder="0.00"
					value={draft.price}
					oninput={(event) => set('price', event.currentTarget.value)}
				/>
			</Field>
			<!-- The percentage is TPT's own and is stated only when TPT has been
			     asked. A literal here read as a rule of theirs on a page whose
			     banner said their rules could not be read. -->
			<Field
				label="Multiple Licenses"
				id="draft-additional-licence"
				required={!optional}
				hint={form === null
					? 'Set the price for extra copies.'
					: `We suggest ${form.limits.additional_licence_percentage}% of the price.`}
			>
				<input
					id="draft-additional-licence"
					type="text"
					inputmode="decimal"
					aria-required={optional ? undefined : 'true'}
					placeholder="0.00"
					value={draft.additionalLicence}
					oninput={(event) => set('additionalLicence', event.currentTarget.value)}
				/>
			</Field>
			<Field
				label="Bundle Discount Price"
				id="draft-bundle-discount"
				hint="Enter the price for the whole bundle."
			>
				<input
					id="draft-bundle-discount"
					type="text"
					inputmode="decimal"
					placeholder="0.00"
					value={draft.bundleDiscount}
					oninput={(event) => set('bundleDiscount', event.currentTarget.value)}
				/>
			</Field>
		</div>
	{/if}
</FormSection>
