<script lang="ts">
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { api, ApiFailure, type ConnectionView } from '$lib/api';
	import { present } from '$lib/connection-status';
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

<h1 class="mb-4 text-xl font-semibold">Connections</h1>

{#if needsReauth.length > 0}
	<div class="mb-4 rounded border-2 border-orange-400 bg-orange-50 p-4">
		<h2 class="font-semibold text-orange-900">Waiting on you</h2>
		<p class="text-sm text-orange-800">
			{needsReauth.length === 1 ? 'A connection needs' : `${needsReauth.length} connections need`}
			re-linking before queued work can continue. Nothing is lost; items are
			paused with their identities intact and resume exactly where they stopped.
		</p>
	</div>
{/if}

{#if listing.isPending}
	<p class="text-slate-500">Loading…</p>
{:else if listing.isError}
	<p class="text-slate-500">The connections could not be read.</p>
{:else if connections.length === 0}
	<p class="text-slate-500">No marketplace connections yet.</p>
{:else}
	<ul class="divide-y divide-slate-100 rounded border border-slate-200 bg-white">
		{#each connections as connection (connection.id)}
			<li class="flex items-center gap-4 px-4 py-3">
				<span class="font-medium">{connection.marketplace}</span>
				<span
					class="rounded px-2 py-0.5 text-xs {present(connection.status).tone}"
					title={present(connection.status).explanation}
				>
					{connection.status}
				</span>
				<span class="text-xs text-slate-500">
					{present(connection.status).explanation}
				</span>
				<span class="grow"></span>
				{#if connection.state !== 'revoked'}
					<button
						class="rounded border border-red-300 px-3 py-1 text-sm text-red-700 disabled:opacity-50"
						disabled={revoking.isPending && revoking.variables === connection.id}
						onclick={() => revoke(connection)}
					>
						{revoking.isPending && revoking.variables === connection.id
							? 'Revoking…'
							: 'Revoke'}
					</button>
				{/if}
			</li>
		{/each}
	</ul>
	<p class="mt-3 text-xs text-slate-500">
		Deleting a connection record is not offered yet; revocation destroys the
		credential, which is the part that matters.
	</p>
{/if}
