<script lang="ts">
	import Icon from '$lib/Icon.svelte';

	let {
		who,
		stopping,
		refusal,
		onStop
	}: {
		who: string;
		stopping: boolean;
		/** What went wrong ending the impersonation, shown in the banner rather
		 *  than as a toast: a toast expires and this state does not. */
		refusal: string | null;
		onStop: () => void;
	} = $props();
</script>

<div class="impersonating" role="alert">
	<span class="mark"><Icon name="copy" size={14} /></span>
	<span class="said">
		You are signed in as <b>{who}</b>. Anything you do here is done as them.
	</span>
	{#if refusal}<span class="refused">{refusal}</span>{/if}
	<button type="button" onclick={onStop} disabled={stopping}>
		{stopping ? 'Stopping…' : 'Stop'}
	</button>
</div>
