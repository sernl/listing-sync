<script lang="ts">
	let {
		checked = $bindable(false),
		label,
		disabled = false,
		onchange
	}: {
		checked?: boolean;
		label: string;
		disabled?: boolean;
		onchange?: (value: boolean) => void;
	} = $props();
</script>

<label class="toggle">
	<input
		type="checkbox"
		bind:checked
		{disabled}
		onchange={(event) => onchange?.(event.currentTarget.checked)}
	/>
	<span class="track" aria-hidden="true"></span>
	{label}
</label>
