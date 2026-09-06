<script lang="ts">
	// The listing's pictures: the thumbnail drawn from the file, the mode
	// choice, and TPT's own four slots. Extracted from `ResourceForm` when that
	// form gained a second mode, so neither the create nor the edit path carries
	// a hundred lines of image markup it does not read.
	import StatusPill from '$lib/StatusPill.svelte';
	import type { FormVocabularyView } from '$lib/api';
	import type { ThumbnailSlot } from '$lib/tpt-form';

	let {
		form,
		mode,
		slots,
		coverUrl,
		gigabytes,
		onMode,
		onPick,
		onClear
	}: {
		form: FormVocabularyView;
		/** `draft.thumbnailMode`: `1` auto-generate, `2` upload now, `3` later. */
		mode: string;
		slots: readonly ThumbnailSlot[];
		/** Where the thumbnail this listing already has can be fetched, or
		 *  `null` where there is none yet. The create reads it by the upload's
		 *  own handle, because no product exists to address it through; the edit
		 *  reads the saved resource's cover. */
		coverUrl: string | null;
		gigabytes: (bytes: number) => string;
		onMode: (id: string) => void;
		onPick: (index: number, file: File) => void;
		onClear: (index: number) => void;
	} = $props();

	// The cover URL whose bytes would not draw. Held as the URL rather than a
	// flag so a later cover clears it by being a different URL, which a flag
	// would need an effect to do.
	let unshowable = $state<string | null>(null);
	const cover = $derived(coverUrl === unshowable ? null : coverUrl);
</script>

<fieldset class="res-choices res-stack">
	<legend>Thumbnails</legend>
	<p class="res-note">
		The pictures buyers see first. The four slots appear only under "Upload thumbnails now",
		exactly as they do on TPT.
	</p>

	<!-- What the seller can actually be shown, and nothing else. The cover is
	     made on our server from the file and served back by its own route, so
	     it is the thing itself rather than a stand-in: a box captioned
	     "thumbnail" showing something else is the defect this note exists to
	     avoid. A broken-image glyph is one of those stand-ins, so bytes that
	     will not draw fall back to the empty tile below, as a slot's own picture
	     already does. -->
	<div class="res-thumb">
		{#if cover !== null}
			<img
				class="res-thumb-img"
				src={cover}
				alt="What buyers see at the top of this listing"
				onerror={() => (unshowable = coverUrl)}
			/>
			<p class="res-note">
				Made from your file when you uploaded it. This is the thumbnail buyers see first.
			</p>
		{:else}
			<div class="res-thumb-none">
				<span class="res-thumb-mark">Nothing yet</span>
			</div>
			<p class="res-note">Upload a file and the thumbnail appears here.</p>
		{/if}
	</div>
	<div class="res-choices">
		{#each form.thumbnail_modes as option (option.id)}
			<label>
				<input
					type="radio"
					name="thumbnail-mode"
					checked={mode === option.id}
					onchange={() => onMode(option.id)}
				/>
				{option.label}
			</label>
		{/each}
	</div>
	{#if mode === '2'}
		<div class="res-slots">
			{#each slots as slot, index (index)}
				<div class="res-slot">
					<b>{index === 0 ? 'Main Cover' : 'Thumbnail (Optional)'}</b>
					{#if slot.local !== null}
						<!-- The picture the seller can be shown. For a slot seeded from
						     a stored hash this is that blob's own route, which serves
						     PNG, JPEG, GIF and WebP, decided from the bytes. The
						     fallback stays: a format outside those four, or bytes the
						     store cannot produce, leaves the slot saying it is stored
						     rather than showing a broken image. -->
						<img
							class="res-slot-img"
							src={slot.local}
							alt=""
							onerror={(event) => ((event.currentTarget as HTMLImageElement).hidden = true)}
						/>
						{#if slot.sending}
							<span class="res-note">Uploading…</span>
						{:else if slot.handle !== null}
							<StatusPill tone="ok" label="stored" />
						{/if}
						<button type="button" class="res-slot-drop" onclick={() => onClear(index)}>
							Remove
						</button>
					{:else}
						<label class="res-slot-pick">
							<span class="res-note">Choose a picture</span>
							<span class="res-note">
								Up to {gigabytes(form.limits.thumbnail.max_size_bytes)}
							</span>
							<input
								type="file"
								accept="image/*"
								onchange={(event) => {
									const chosen = event.currentTarget.files?.[0];
									if (chosen) {
										onPick(index, chosen);
									}
									event.currentTarget.value = '';
								}}
							/>
						</label>
					{/if}
					{#if slot.refusal !== null}
						<span class="res-pick-why bad">{slot.refusal}</span>
					{/if}
				</div>
			{/each}
		</div>
		<p class="res-foot">
			These four slots match TPT's own layout. Each picture is saved as you choose it, and only
			the ones marked stored travel with the listing.
		</p>
	{/if}
</fieldset>
