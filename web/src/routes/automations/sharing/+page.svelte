<script lang="ts">
	import { api, type ConnectionView, type DeviceView } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import { remember, remembered } from '$lib/dismissal';
	import Button from '$lib/Button.svelte';
	import Field from '$lib/Field.svelte';
	import type { Marketplace } from '$lib/generated/vocab';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Toggle from '$lib/Toggle.svelte';
	import MarketplaceList from '$lib/pages/automations/MarketplaceList.svelte';
	import { heldSelection, marketplaceRows } from '$lib/pages/automations/marketplace-list';
	import { MARKETPLACE_WORD } from '$lib/platforms';
	import { columnCopy, panelCopy, readState } from '$lib/pages/automations/read-state';
	import {
		DISABLED_REASON,
		NOT_BUILT_BODY,
		NOT_BUILT_TITLE,
		NO_DEVICE_BODY,
		NO_DEVICE_TITLE,
		SETTINGS_ARE_A_PREVIEW,
		WHEN_OPTIONS,
		timezoneLine,
		timezoneName
	} from '$lib/pages/automations/sharing';
	import '$lib/pages/automations/automations.css';

	let connections = $state<ConnectionView[]>([]);
	let connectionsLoaded = $state(false);
	let connectionsFailed = $state(false);

	let devices = $state<DeviceView[]>([]);
	// Whether the machine list was read at all. A seller whose devices could
	// not be listed is not a seller with no machine, and the banner telling
	// them to install the app is the wrong thing to show in that case.
	let devicesRead = $state(false);

	let held = $state<Marketplace | null>(null);

	$effect(() => {
		void api
			.connections()
			.then((held) => {
				connections = held;
				connectionsFailed = false;
			})
			.catch(() => {
				connections = [];
				connectionsFailed = true;
			})
			.finally(() => (connectionsLoaded = true));
		void api
			.devices()
			.then((view) => {
				devices = view.devices;
				devicesRead = true;
			})
			.catch(() => (devicesRead = false));
	});

	const rows = $derived(marketplaceRows(connections));
	const read = $derived(readState(connectionsLoaded, connectionsFailed, rows));
	const copy = $derived(panelCopy(read, 'Sharing'));
	// The resolved selection rather than the raw click, so the row the column
	// highlights and the marketplace the card describes cannot come apart when
	// a refetch drops the marketplace that was chosen.
	const selected = $derived(heldSelection(rows, held));
	const shown = $derived(rows.find((row) => row.marketplace === selected) ?? null);
	const noDevice = $derived(devicesRead && devices.length === 0);
	const zone = timezoneLine(timezoneName());

	/** Closed for good once closed: the sentence is about the product and
	 *  does not change, so meeting it again on every visit is noise. */
	let notBuiltShown = $state(!remembered('sharing.not-built'));
</script>

<div class="page">
	<PageHead
		icon="share-2"
		title="Marketplace Sharing"
		description="Publish one resource to every marketplace you are connected to, in one scheduled action."
	/>

	{#if notBuiltShown}
		<Banner
			tone="info"
			title={NOT_BUILT_TITLE}
			onDismiss={() => {
				notBuiltShown = false;
				remember('sharing.not-built');
			}}
		>
			{NOT_BUILT_BODY}
		</Banner>
	{/if}

	<div class="auto-body">
		<MarketplaceList {rows} {selected} onselect={(marketplace) => (held = marketplace)}
			empty={columnCopy(read) ?? ''} />

		<div class="auto-right">
			{#if copy !== null}
				<Panel title={copy.title}>
					{#if copy.body}<p class="quiet">{copy.body}</p>{/if}
				</Panel>
			{:else if shown !== null}
				<Panel title={MARKETPLACE_WORD[shown.marketplace]} description={SETTINGS_ARE_A_PREVIEW}>
					<div class="rule-row">
						<span>Active rules (0)</span>
						<span>None yet</span>
					</div>

					<div class="set-grid">
						<div class="set-toggle">
							<Toggle label="Include this marketplace" checked={false} disabled />
						</div>

						<Field label="When" id="share-when" hint={DISABLED_REASON}>
							<select id="share-when" disabled>
								{#each WHEN_OPTIONS as option (option)}
									<option>{option}</option>
								{/each}
							</select>
						</Field>

						<Field label="Time" id="share-time" hint={`${zone} ${DISABLED_REASON}`}>
							<input id="share-time" type="time" value="09:00" disabled />
						</Field>

						<Field
							label="Only resources with these labels"
							id="share-labels"
							hint={DISABLED_REASON}
						>
							<select id="share-labels" disabled>
								<option>Every resource</option>
							</select>
						</Field>
					</div>

					{#if noDevice}
						<Banner tone="warn" title={NO_DEVICE_TITLE}>
							{NO_DEVICE_BODY}
							{#snippet action()}
								<Button href="/marketplaces" tier="outline" small>Downloads</Button>
							{/snippet}
						</Banner>
					{/if}

					<div class="set-foot">
						<Button tier="primary" disabled reason={DISABLED_REASON}>Save schedule</Button>
					</div>
				</Panel>
			{/if}
		</div>
	</div>
</div>
