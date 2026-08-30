<script lang="ts">
	import type { AxisControl } from '$lib/authoring';

	let {
		control,
		chosen,
		onChange
	}: {
		control: AxisControl;
		chosen: string[];
		onChange: (values: string[]) => void;
	} = $props();

	const id = $props.id();

	const values = $derived(control.values ?? []);
	const overCap = $derived(control.cap !== null && chosen.length > control.cap);

	function pick(event: Event & { currentTarget: HTMLSelectElement }) {
		onChange(
			control.multiple
				? [...event.currentTarget.selectedOptions].map((option) => option.value)
				: [event.currentTarget.value].filter((value) => value.length > 0)
		);
	}
</script>

{#if control.unwritableReason !== null}
	<p class="disclosure">
		<b>{control.native}</b> — {control.unwritableReason}.
	</p>
{:else if control.vocabulary !== 'closed' || values.length === 0}
	<p class="disclosure">
		<b>{control.native}</b> — this platform's values for {control.axis} are
		{control.vocabulary === 'closed_uncaptured'
			? 'a closed set we have not captured, so there is no picker to offer'
			: 'not a set we hold, so there is no picker to offer'}. The projection fills the field from
		your taxonomy.
	</p>
{:else}
	<label class="field" for={id}>
		{control.native}
		<span class="hint">
			{control.axis}, {control.multiple ? 'several' : 'one'}{control.cap === null
				? ''
				: `, up to ${control.cap}`}.
		</span>
		<select
			{id}
			multiple={control.multiple}
			size={control.multiple ? Math.min(values.length, 6) : undefined}
			onchange={pick}
		>
			{#if !control.multiple}
				<option value="" selected={chosen.length === 0}>Not stated</option>
			{/if}
			{#each values as value (value.id)}
				<option value={value.id} selected={chosen.includes(value.id)}>{value.label}</option>
			{/each}
		</select>
		<span class="hint">
			{#if control.nonDelegableReason !== null}
				Yours to decide, always: {control.nonDelegableReason}.
			{:else}
				Yours to decide. Handing an axis to a best-fit choice needs a setting we do not model
				yet, so every axis is answered by you today.
			{/if}
		</span>
		{#if overCap}
			<span class="refusal">
				{control.native} takes at most {control.cap} on this platform.
			</span>
		{/if}
	</label>
{/if}
