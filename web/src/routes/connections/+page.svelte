<script lang="ts">
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { api, ApiFailure, type ConnectionView } from '$lib/api';
	import { present } from '$lib/connection-status';
	import { agoLabel } from '$lib/elapsed';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import { queryKeys } from '$lib/query';
	import { toast } from '$lib/toast';

	const queryClient = useQueryClient();

	const listing = createQuery(() => ({
		queryKey: queryKeys.connections,
		queryFn: () => api.connections()
	}));

	const connections = $derived(listing.data?.connections ?? []);
	const needsReauth = $derived(connections.filter((c) => c.state === 'needs_reauth'));

	const revoking = createMutation(() => ({
		mutationFn: (connection: string) => api.revoke(connection),
		onSuccess: async (done: { elapsed_ms: number }) => {
			toast('info', `Revoked in ${done.elapsed_ms}ms.`);
			await queryClient.invalidateQueries({ queryKey: queryKeys.connections });
		},
		onError: (failure: Error) => {
			toast(
				'error',
				failure instanceof ApiFailure && failure.code() === 'broker_unavailable'
					? 'The credential broker is not running; nothing was revoked.'
					: 'The revoke did not complete.'
			);
		}
	}));

	function revoke(connection: ConnectionView) {
		const sure = confirm(
			'Revoke this connection? The stored credential is destroyed and every ' +
				'queued item for it pauses until you re-link.'
		);
		if (!sure) {
			return;
		}
		revoking.mutate(connection.id);
	}
</script>

<div class="page">
	<PageHead
		icon="⚲"
		title="Connections"
		description="Your marketplace sign-ins and their health."
	/>

	{#if needsReauth.length > 0}
		<div class="attn">
			<div class="t">Waiting on you</div>
			<p>
				{needsReauth.length === 1
					? 'A connection needs'
					: `${needsReauth.length} connections need`}
				re-linking before queued work can continue. Nothing is lost; items are paused with
				their identities intact and resume exactly where they stopped.
			</p>
		</div>
	{/if}

	<Panel>
		{#if listing.isPending}
			<p class="quiet">Loading…</p>
		{:else if listing.isError}
			<p class="quiet">The connections could not be read.</p>
		{:else if connections.length === 0}
			<div class="placeholder">
				<span class="big" aria-hidden="true">⚲</span>
				<b>No marketplace connections yet.</b>
				<p>
					A connection is your sign-in to one marketplace, held on our side so the engine can
					work without you. Once one is linked, its health shows here and in the top bar.
				</p>
			</div>
		{:else}
			{#each connections as connection (connection.id)}
				{@const shown = present(connection.status)}
				<div class="row">
					<span class="t">{connection.marketplace}</span>
					<span class="pill {shown.tone}">{connection.status}</span>
					<span class="s">{shown.explanation}</span>
					<span class="grow"></span>
					<span class="when">linked {agoLabel(connection.created_at, Date.now())}</span>
					{#if connection.state !== 'revoked'}
						<button
							class="btn small danger"
							disabled={revoking.isPending && revoking.variables === connection.id}
							onclick={() => revoke(connection)}
						>
							{revoking.isPending && revoking.variables === connection.id
								? 'Revoking…'
								: 'Revoke'}
						</button>
					{/if}
				</div>
			{/each}
			<p class="foot-note">
				Deleting a connection record is not offered yet; revocation destroys the credential,
				which is the part that matters.
			</p>
		{/if}
	</Panel>
</div>
