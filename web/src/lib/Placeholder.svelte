<script lang="ts">
	let {
		icon,
		headline,
		body
	}: {
		icon: string;
		headline: string;
		body: string;
	} = $props();
</script>

<div class="placeholder">
	<span class="big" aria-hidden="true">{icon}</span>
	<b>{headline}</b>
	<p>{body}</p>
</div>
