<script lang="ts">
	import { api, ApiFailure, type ConnectionView } from '$lib/api';
	import { toast } from '$lib/toast';

	let connections = $state<ConnectionView[]>([]);
	let loaded = $state(false);
	let revoking = $state<string | null>(null);

	async function refetch() {
		connections = (await api.connections()).connections;
		loaded = true;
	}

	$effect(() => {
		void refetch();
	});

	const needsReauth = $derived(connections.filter((c) => c.state === 'needs_reauth'));

	async function revoke(connection: ConnectionView) {
		const sure = confirm(
			'Revoke this connection? The stored credential is destroyed and every ' +
				'queued item for it pauses until you re-link.'
		);
		if (!sure) {
			return;
		}
		revoking = connection.id;
		try {
			const done = await api.revoke(connection.id);
			toast('info', `Revoked in ${done.elapsed_ms}ms.`);
			await refetch();
		} catch (failure) {
			const message =
				failure instanceof ApiFailure && failure.code() === 'broker_unavailable'
					? 'The credential broker is not running; nothing was revoked.'
					: 'The revoke did not complete.';
			toast('error', message);
		} finally {
			revoking = null;
		}
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

{#if !loaded}
	<p class="text-slate-500">Loading…</p>
{:else if connections.length === 0}
	<p class="text-slate-500">No marketplace connections yet.</p>
{:else}
	<ul class="divide-y divide-slate-100 rounded border border-slate-200 bg-white">
		{#each connections as connection (connection.id)}
			<li class="flex items-center gap-4 px-4 py-3">
				<span class="font-medium">{connection.marketplace}</span>
				<span
					class="rounded px-2 py-0.5 text-xs
						{connection.state === 'linked'
						? 'bg-emerald-100 text-emerald-800'
						: connection.state === 'needs_reauth'
							? 'bg-orange-100 text-orange-800'
							: 'bg-slate-100 text-slate-700'}"
				>
					{connection.state}
				</span>
				<span class="grow"></span>
				{#if connection.state !== 'revoked'}
					<button
						class="rounded border border-red-300 px-3 py-1 text-sm text-red-700 disabled:opacity-50"
						disabled={revoking === connection.id}
						onclick={() => revoke(connection)}
					>
						{revoking === connection.id ? 'Revoking…' : 'Revoke'}
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
