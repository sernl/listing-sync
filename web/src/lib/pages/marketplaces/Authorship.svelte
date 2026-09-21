<script lang="ts">
	import { useQueryClient } from '@tanstack/svelte-query';
	import { ApiFailure, api } from '$lib/api';
	import Button from '$lib/Button.svelte';
	import Field from '$lib/Field.svelte';
	import type { MarketplaceRow } from '$lib/devices-view';
	import { agoLabel } from '$lib/elapsed';
	import type { Marketplace } from '$lib/generated/vocab';
	import { queryKeys } from '$lib/query';
	import { CARD_NAME, type LiveRead } from './view';

	let {
		rows,
		now,
		read
	}: {
		rows: readonly MarketplaceRow[];
		now: number;
		/** Whether the connection read landed. A declaration is read off that
		 *  same list, so an unread row and a seller who has declared nothing are
		 *  the same shape; saying "Not declared" on the first would state a fact
		 *  we never received, on the one control whose absence fails a send. */
		read: LiveRead['state'];
	} = $props();

	const queryClient = useQueryClient();

	// One form at a time, opened on the row it belongs to, so a name cannot be
	// typed against a marketplace the seller is not looking at.
	let openOn = $state<Marketplace | null>(null);
	let typed = $state('');
	let saving = $state(false);
	let refusal = $state<string | null>(null);

	/** The standing declaration on a row, or null where the seller has made
	 *  none. Read off the row's own `authorship` rather than off `connection`,
	 *  which is null on the device branch this control belongs to. */
	function standing(row: MarketplaceRow): { name: string; attested_at: number } | null {
		const held = row.authorship;
		return held !== undefined && held.state === 'declared' ? held : null;
	}

	function open(row: MarketplaceRow) {
		openOn = row.marketplace;
		typed = standing(row)?.name ?? '';
		refusal = null;
	}

	async function declare(marketplace: Marketplace) {
		const name = typed.trim();
		if (name.length === 0) {
			return;
		}
		saving = true;
		refusal = null;
		try {
			await api.declareAuthorship(marketplace, name);
			await queryClient.invalidateQueries({ queryKey: queryKeys.connections });
			openOn = null;
		} catch (failure) {
			refusal =
				failure instanceof ApiFailure ? failure.message : 'That declaration was not saved.';
		} finally {
			saving = false;
		}
	}
</script>

<section id="copyright">
	<div class="mp-sect">
		<h2>Who made this work</h2>
		<p>
			Declare who holds the copyright in your work, once per marketplace.
			<a class="mp-guide" href="/guides/copyright">Who holds the copyright</a>
		</p>
	</div>

	{#if read !== 'read'}
		<p class="quiet">
			{read === 'pending'
				? 'Reading your declarations…'
				: 'Your declarations could not be read, so reload to try again.'}
		</p>
	{:else}
		<div class="mp-card">
			{#each rows as row (row.marketplace)}
			<div class="mp-machine">
				<div class="who">
					<span class="t">{CARD_NAME[row.marketplace]}</span>
					<span class="act">
						<Button tier="outline" small icon="pencil" onclick={() => open(row)}>
							{standing(row) ? 'Change' : 'Declare'}
						</Button>
					</span>
				</div>
				{#if standing(row)}
					<p class="spec">
						Declared by {standing(row)?.name}, {agoLabel(
							standing(row)?.attested_at ?? now,
							now
						)}.
					</p>
				{:else if row.marketplace === 'Tpt'}
					<p class="mp-warned">
						Not declared, so anything sent to TPT fails until you declare it.
					</p>
				{:else}
					<p class="spec">
						Not declared, and {CARD_NAME[row.marketplace]} does not ask.
					</p>
				{/if}

				{#if openOn === row.marketplace}
					<div class="mp-held">
						<Field
							label="Your name, as the copyright holder"
							id="mp-authorship-{row.marketplace}"
						>
							<input
								id="mp-authorship-{row.marketplace}"
								type="text"
								maxlength="200"
								placeholder="The name to show as the copyright holder"
								disabled={saving}
								bind:value={typed}
							/>
						</Field>
						<div class="one">
							<Button
								tier="additive"
								small
								disabled={saving || typed.trim().length === 0}
								reason={saving
									? 'The declaration is being saved.'
									: typed.trim().length === 0
										? 'Type the name first.'
										: undefined}
								onclick={() => void declare(row.marketplace)}
							>
								{saving ? 'Saving…' : 'Save'}
							</Button>
							<Button
								tier="outline"
								small
								disabled={saving}
								reason={saving ? 'The declaration is being saved.' : undefined}
								onclick={() => (openOn = null)}
							>
								Cancel
							</Button>
						</div>
						{#if refusal !== null}<p class="mp-warned">{refusal}</p>{/if}
					</div>
				{/if}
			</div>
			{/each}
		</div>
	{/if}
</section>
