<script lang="ts">
	// Why a file's bytes could not be read just now, under the control that
	// asked. An offline device carries the Explain saying how an imported file
	// reaches a browser; every other refusal is its sentence alone.
	import Explain from '$lib/Explain.svelte';
	import { STREAM_EXPLAINED, reachSentence, type Unreachable } from './file-viewer';

	let { reach }: { reach: Unreachable } = $props();
</script>

<div class="res-file-say res-file-copy" role="status">
	<span>{reachSentence(reach)}</span>
	{#if reach.kind === 'offline'}
		<Explain title="Where your file is" label="Explain">
			{#each STREAM_EXPLAINED as line (line)}<p>{line}</p>{/each}
		</Explain>
	{/if}
</div>
