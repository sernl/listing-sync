<script lang="ts">
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { api, ApiFailure, type SeasonName, type SitePatch, type SiteView } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import Explain from '$lib/Explain.svelte';
	import Field from '$lib/Field.svelte';
	import FlowStep from '$lib/FlowStep.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Toggle from '$lib/Toggle.svelte';
	import { queryKeys } from '$lib/query';
	import { toast } from '$lib/toast';
	import '$lib/flow.css';

	const queryClient = useQueryClient();

	// The operator's own read: uncached on the server, so a change made here
	// shows here at once rather than after the public read's minute.
	const site = createQuery(() => ({
		queryKey: queryKeys.adminSite,
		queryFn: () => api.adminSite()
	}));

	// Drafts, seeded from the server once it answers and again after each save.
	let maintenanceOn = $state(false);
	let message = $state('');
	let season = $state<SeasonName>('none');
	let from = $state('');
	let until = $state('');
	let bannerText = $state('');
	let bannerHref = $state('');
	// Plain, not `$state`: it is only compared inside the effect, and a deep
	// proxy would never equal the query's object, so the effect would loop.
	let seeded: SiteView | null = null;

	$effect(() => {
		const view = site.data;
		if (view === undefined || view === seeded) {
			return;
		}
		seeded = view;
		maintenanceOn = view.maintenance.on;
		message = view.maintenance.message ?? '';
		season = view.theme.name;
		from = view.theme.from ?? '';
		until = view.theme.until ?? '';
		bannerText = view.banner?.text ?? '';
		bannerHref = view.banner?.href ?? '';
	});

	const saving = createMutation(() => ({
		mutationFn: (patch: { body: SitePatch; said: string }) => api.updateSite(patch.body),
		onSuccess: (view: SiteView, patch: { body: SitePatch; said: string }) => {
			queryClient.setQueryData(queryKeys.adminSite, view);
			toast('info', patch.said);
		},
		onError: (failure: Error) =>
			toast('error', failure instanceof ApiFailure ? failure.message : 'That change was not saved.')
	}));

	function saveMaintenance(event: SubmitEvent) {
		event.preventDefault();
		if (
			maintenanceOn &&
			!seeded?.maintenance.on &&
			!confirm(
				'Turn maintenance on?\n\nEveryone except admins will see the maintenance page instead of the site and the console.'
			)
		) {
			return;
		}
		saving.mutate({
			body: { maintenance: { on: maintenanceOn, message: message.trim() || null } },
			said: maintenanceOn ? 'Maintenance is on.' : 'Maintenance is off.'
		});
	}

	function saveTheme(event: SubmitEvent) {
		event.preventDefault();
		saving.mutate({
			body: { theme: { name: season, from: from || null, until: until || null } },
			said: season === 'none' ? 'No seasonal theme.' : 'Seasonal theme saved.'
		});
	}

	function saveBanner(event: SubmitEvent) {
		event.preventDefault();
		saving.mutate({
			body: { banner: { text: bannerText, href: bannerHref } },
			said: 'Banner saved.'
		});
	}

	function removeBanner() {
		saving.mutate({ body: { banner: null }, said: 'Banner removed.' });
	}

	const SEASONS: readonly { value: SeasonName; label: string }[] = [
		{ value: 'none', label: 'None' },
		{ value: 'halloween', label: 'Halloween' },
		{ value: 'christmas', label: 'Christmas' }
	];

	const themeSummary = $derived.by(() => {
		const theme = site.data?.theme;
		if (theme === undefined || theme.name === 'none') {
			return 'No seasonal theme.';
		}
		const name = SEASONS.find((entry) => entry.value === theme.name)?.label ?? theme.name;
		const days = `${theme.from ?? 'now'} to ${theme.until ?? 'no end date'}`;
		return `${name}, ${days}${theme.active ? ', showing today' : ', not showing today'}.`;
	});
</script>

<div class="page flow-page">
	<PageHead
		icon="sliders-horizontal"
		title="Site"
		description="Switches that change the whole site for everyone: maintenance, the seasonal theme and the banner."
	/>

	{#if site.isPending}
		<p class="quiet">Loading the site switches…</p>
	{:else if site.isError}
		<Banner tone="bad">We could not load the site switches. Try reloading the page.</Banner>
	{:else}
		<div class="flow">
			<FlowStep
				n={1}
				id="maintenance"
				title="Maintenance"
				hint="Show a maintenance page instead of the site and the console."
				summary={site.data.maintenance.on ? 'Maintenance is on.' : 'Maintenance is off.'}
				done={!site.data.maintenance.on}
			>
				{#snippet aside()}
					<Explain title="How maintenance works" label="How it works">
						<p>
							While it is on, every page of the site and the console answers with the maintenance
							page, and search engines are told to come back later.
						</p>
						<p>
							Admins are not affected, so you can keep working and turn it off here. Signing in
							and the status page stay open for everyone.
						</p>
					</Explain>
				{/snippet}
				<form class="form" onsubmit={saveMaintenance}>
					<Toggle label="Maintenance on" bind:checked={maintenanceOn} />
					<Field
						label="When you expect to be back"
						id="maintenance-message"
						hint="Shown on the maintenance page. Leave empty to say “We expect to be back shortly.”"
					>
						<input
							id="maintenance-message"
							type="text"
							placeholder="Back by 3pm UTC today."
							bind:value={message}
						/>
					</Field>
					<div class="actions">
						<Button tier="primary" type="submit" disabled={saving.isPending} reason="Saving…">
							Save maintenance
						</Button>
						<a class="btn" href="/maintenance/" target="_blank" rel="noopener">Preview the page</a>
					</div>
				</form>
			</FlowStep>

			<FlowStep
				n={2}
				id="theme"
				title="Seasonal theme"
				hint="Decorations on the public site between two dates."
				summary={themeSummary}
				done={site.data.theme.active}
			>
				{#snippet aside()}
					<Explain title="How the seasonal theme works" label="How it works">
						<p>
							The theme shows on the public site from the first day to the last day, by UTC
							dates, and a small mark appears in the console’s top bar. Leave a date empty to
							start now or run with no end.
						</p>
						<p>The preview shows the decorations whatever the dates say.</p>
					</Explain>
				{/snippet}
				<form class="form" onsubmit={saveTheme}>
					<Field label="Theme" id="theme-name">
						<select id="theme-name" bind:value={season}>
							{#each SEASONS as entry (entry.value)}
								<option value={entry.value}>{entry.label}</option>
							{/each}
						</select>
					</Field>
					<div class="site-dates">
						<Field label="From" id="theme-from">
							<input id="theme-from" type="date" bind:value={from} />
						</Field>
						<Field label="Until" id="theme-until">
							<input id="theme-until" type="date" bind:value={until} />
						</Field>
					</div>
					<div class="actions">
						<Button tier="primary" type="submit" disabled={saving.isPending} reason="Saving…">
							Save theme
						</Button>
						{#if season !== 'none'}
							<a class="btn" href="/?season={season}" target="_blank" rel="noopener">Preview the site</a>
						{/if}
					</div>
				</form>
			</FlowStep>

			<FlowStep
				n={3}
				id="banner"
				title="Banner"
				hint="One line above the public site’s header, with a link."
				summary={site.data.banner === null ? 'No banner.' : site.data.banner.text}
				done={site.data.banner !== null}
			>
				<form class="form" onsubmit={saveBanner}>
					<Field label="Text" id="banner-text" hint="At most 140 characters.">
						<input id="banner-text" type="text" required bind:value={bannerText} />
					</Field>
					<Field
						label="Link"
						id="banner-href"
						hint="A path on this site, like /pricing/, or an https address."
					>
						<input id="banner-href" type="text" required bind:value={bannerHref} />
					</Field>
					<div class="actions">
						<Button tier="primary" type="submit" disabled={saving.isPending} reason="Saving…">
							Save banner
						</Button>
						{#if site.data.banner !== null}
							<Button danger onclick={removeBanner}>Remove banner</Button>
						{/if}
					</div>
				</form>
			</FlowStep>
		</div>
	{/if}
</div>

<style>
	.site-dates {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
		gap: 12px;
	}

	.site-dates input {
		border: 1px solid var(--line);
		background: var(--card);
		border-radius: var(--r-field);
		padding: 8px 11px;
		min-height: var(--control-h);
		font: inherit;
		font-weight: 400;
		color: var(--ink);
		min-width: 0;
	}

	.site-dates input:focus-visible {
		border-color: var(--primary);
	}
</style>
