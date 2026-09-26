<script lang="ts">
	import { useQueryClient } from '@tanstack/svelte-query';
	import { ApiFailure, api } from '$lib/api';
	import Button from '$lib/Button.svelte';
	import Field from '$lib/Field.svelte';
	import MarketplaceMark from '$lib/MarketplaceMark.svelte';
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
				failure instanceof ApiFailure ? failure.message : 'Your name was not saved. Try again.';
		} finally {
			saving = false;
		}
	}
</script>

<div id="copyright" class="mp-authors">
	{#if read !== 'read'}
		<p class="quiet">
			{read === 'pending'
				? 'Loading…'
				: 'We could not load this. Reload the page to try again.'}
		</p>
	{:else}
		<div class="flow-list">
			{#each rows as row (row.marketplace)}
				{@const held = standing(row)}
				<div class="flow-item mp-author" class:mp-owed={held === null && row.marketplace === 'Tpt'}>
					<MarketplaceMark marketplace={row.marketplace} size={24} />
					<span class="flow-item-main">
						{#if held}
							<span class="flow-item-title">{held.name}</span>
							<span class="flow-item-line">Declared {agoLabel(held.attested_at, now)}</span>
						{:else if row.marketplace === 'Tpt'}
							<span class="flow-item-title mp-warned">Not declared</span>
							<span class="flow-item-line">TPT refuses anything you publish until you declare it.</span>
						{:else}
							<span class="flow-item-title">Not declared</span>
							<span class="flow-item-line">{CARD_NAME[row.marketplace]} does not need this.</span>
						{/if}
					</span>
					<span class="flow-item-acts">
						<Button
							tier={held === null && row.marketplace === 'Tpt' ? 'primary' : 'outline'}
							small
							icon="pencil"
							onclick={() => open(row)}
						>
							{held ? 'Change' : 'Declare'}
						</Button>
					</span>

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
							<div class="flow-actions">
								<Button
									tier="additive"
									small
									disabled={saving || typed.trim().length === 0}
									reason={saving
										? 'Saving…'
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
									reason={saving ? 'Saving…' : undefined}
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
</div>
