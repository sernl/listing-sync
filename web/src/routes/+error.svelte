<script lang="ts">
	import AuthFrame from '$lib/AuthFrame.svelte';
	import PageError from '$lib/PageError.svelte';
</script>

<!-- A signed-out screen that failed, which the console's own error page is
     not above. Drawn in the signed-out frame, as the screen itself was. -->
<AuthFrame>
	<PageError />
</AuthFrame>
