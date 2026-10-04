<script lang="ts">
	/** What a suspended account sees inside an `auth-card`: in place of the
	 *  console, and on the sign-in page when the exchange is refused for it. It
	 *  stands still rather than sending the browser to sign in, which would
	 *  only be refused again. The sentence is the API's own
	 *  (`account_suspended`), with the address made a link. */
	import Button from '$lib/Button.svelte';

	let { onSignOut }: { onSignOut: () => void } = $props();
</script>

<h1>Account suspended</h1>
<p>This account is suspended. Email <a href="mailto:contact@teachouse.io">contact@teachouse.io</a>.</p>
<Button tier="primary" icon="log-out" onclick={onSignOut}>Sign out</Button>
