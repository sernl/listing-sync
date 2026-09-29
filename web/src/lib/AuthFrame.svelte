<script lang="ts">
	import type { Snippet } from 'svelte';
	import '$lib/styles/auth.css';

	/** The signed-out frame: the wordmark over whatever card stands below it,
	 *  on the signed-out screens' own stylesheet. */
	let { children }: { children: Snippet } = $props();
</script>

<div class="auth">
	<div class="wordmark">
		<img src="/brand/logo.svg" alt="Teachouse" width="220" height="73" />
	</div>
	{@render children()}
</div>
