<script lang="ts">
	import { ApiFailure } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import Field from '$lib/Field.svelte';
	import {
		NAME_MAX_CHARS,
		REASON_MAX_CHARS,
		URL_MAX_CHARS,
		charCount,
		isWebAddress,
		sendMarketplaceRequest
	} from './api';

	let { onclose }: { onclose: () => void } = $props();

	let name = $state('');
	let url = $state('');
	let reason = $state('');
	let sending = $state(false);
	let refusal = $state<string | null>(null);
	/** The name as the server stored it, which is what the confirmation names.
	 *  Not the name in the field: the server trims on the way in, so echoing
	 *  what was typed would thank the seller for a value the row does not
	 *  hold. */
	let thanked = $state<string | null>(null);

	const nameCount = $derived(charCount(name.trim()));
	const reasonCount = $derived(charCount(reason.trim()));

	/** Why the form cannot be sent, or null when it can.
	 *
	 *  The same bounds the server holds, so the seller learns about a field that
	 *  is too long or an address that is not one before the round trip. The
	 *  server checks all of it again, and the refusals only it can make — an
	 *  address already asked about, more requests than one organisation may
	 *  hold — arrive as its own sentence. */
	const blocked = $derived.by(() => {
		if (nameCount === 0) {
			return 'Name the marketplace first.';
		}
		if (nameCount > NAME_MAX_CHARS) {
			return `Keep the name to ${NAME_MAX_CHARS} characters or fewer.`;
		}
		if (url.trim().length === 0) {
			return "Add the marketplace's web address.";
		}
		if (charCount(url.trim()) > URL_MAX_CHARS) {
			return `Keep the web address to ${URL_MAX_CHARS} characters or fewer.`;
		}
		if (!isWebAddress(url.trim())) {
			return 'Enter a full web address, starting with https://.';
		}
		if (reasonCount === 0) {
			return 'Tell us what you sell there.';
		}
		if (reasonCount > REASON_MAX_CHARS) {
			return `Keep this to ${REASON_MAX_CHARS} characters or fewer.`;
		}
		return null;
	});

	async function send(event: SubmitEvent) {
		event.preventDefault();
		if (blocked !== null || sending) {
			return;
		}
		sending = true;
		refusal = null;
		try {
			const stored = await sendMarketplaceRequest({
				name: name.trim(),
				url: url.trim(),
				reason: reason.trim()
			});
			thanked = stored.name;
		} catch (failure) {
			refusal =
				failure instanceof ApiFailure
					? failure.message
					: 'Your request was not sent. Try again in a moment.';
		} finally {
			sending = false;
		}
	}
</script>

<div class="mp-request">
	{#if thanked !== null}
		<Banner tone="ok" onDismiss={onclose} dismissLabel="Close this confirmation">
			Thanks — we have your request for {thanked}.
		</Banner>
	{:else}
		<h3>Request a marketplace</h3>
		<form onsubmit={send}>
			<div class="rows">
				<Field label="Marketplace name" id="mp-name" required>
					<input
						id="mp-name"
						type="text"
						autocomplete="off"
						disabled={sending}
						bind:value={name}
					/>
					<span class="counter" class:over={nameCount > NAME_MAX_CHARS}>
						{nameCount} / {NAME_MAX_CHARS}
					</span>
				</Field>
				<Field label="Web address" id="mp-url" required>
					<input
						id="mp-url"
						type="url"
						placeholder="https://"
						autocomplete="off"
						disabled={sending}
						bind:value={url}
					/>
				</Field>
				<div class="wide">
					<Field label="What do you sell there?" id="mp-reason" required>
						<textarea id="mp-reason" rows="3" disabled={sending} bind:value={reason}
						></textarea>
						<span class="counter" class:over={reasonCount > REASON_MAX_CHARS}>
							{reasonCount} / {REASON_MAX_CHARS}
						</span>
					</Field>
				</div>
			</div>

			<div class="acts">
				<Button
					tier="additive"
					type="submit"
					disabled={sending || blocked !== null}
					reason={sending ? 'Sending your request…' : (blocked ?? undefined)}
				>
					{sending ? 'Sending…' : 'Send request'}
				</Button>
				<Button
					tier="outline"
					disabled={sending}
					reason={sending ? 'Sending your request…' : undefined}
					onclick={onclose}
				>
					Cancel
				</Button>
				<span class="helper">We read every one. We cannot promise a date.</span>
			</div>
		</form>

		{#if refusal !== null}
			<p class="refused">{refusal}</p>
		{/if}
	{/if}
</div>

