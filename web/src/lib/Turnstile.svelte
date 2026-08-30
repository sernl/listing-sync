<script module lang="ts">
	// Cloudflare's documented integration is a script tag, not a package. It is
	// rendered explicitly so the widget appears where the form puts it rather
	// than wherever an implicit scan finds a container, and it is injected only
	// when a site key exists, so an ungated build never reaches Cloudflare.
	const SCRIPT_SRC = 'https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit';

	interface TurnstileApi {
		render(
			container: HTMLElement,
			options: {
				sitekey: string;
				callback: (token: string) => void;
				'error-callback': () => void;
				'expired-callback': () => void;
			}
		): string;
		reset(widget: string): void;
		remove(widget: string): void;
	}

	let loading: Promise<TurnstileApi> | null = null;

	/** Injects the script once per document and resolves with the global it
	 * defines. A rejection is kept, because a script that failed to load will
	 * not load on the next caller either. */
	function loadTurnstile(): Promise<TurnstileApi> {
		loading ??= new Promise<TurnstileApi>((resolve, reject) => {
			const script = document.createElement('script');
			script.src = SCRIPT_SRC;
			script.async = true;
			script.onload = () => {
				const api = (globalThis as unknown as { turnstile?: TurnstileApi }).turnstile;
				if (api) {
					resolve(api);
				} else {
					reject(new Error('the turnstile script defined no global'));
				}
			};
			script.onerror = () => reject(new Error('the turnstile script did not load'));
			document.head.append(script);
		});
		return loading;
	}
</script>

<script lang="ts">
	import { onMount } from 'svelte';
	import { TURNSTILE_SITE_KEY } from '$lib/captcha';

	let { onToken }: { onToken: (token: string | null) => void } = $props();

	let host = $state<HTMLDivElement | null>(null);
	let unavailable = $state(false);
	let api: TurnstileApi | null = null;
	let widget: string | null = null;

	/** Discards the solved token and asks for a fresh one. A Turnstile token is
	 * single-use and is spent by the server's verification, so a submit that
	 * came back refused leaves the widget holding one that cannot be sent
	 * again. */
	export function reset() {
		onToken(null);
		if (api !== null && widget !== null) {
			api.reset(widget);
		}
	}

	onMount(() => {
		const siteKey = TURNSTILE_SITE_KEY;
		if (siteKey === null) {
			return;
		}
		let live = true;
		void loadTurnstile().then(
			(loaded) => {
				if (!live || host === null) {
					return;
				}
				api = loaded;
				widget = loaded.render(host, {
					sitekey: siteKey,
					callback: (token) => onToken(token),
					'error-callback': () => onToken(null),
					'expired-callback': () => onToken(null)
				});
			},
			() => {
				if (live) {
					unavailable = true;
				}
			}
		);
		return () => {
			live = false;
			if (api !== null && widget !== null) {
				api.remove(widget);
			}
		};
	});
</script>

{#if TURNSTILE_SITE_KEY !== null}
	<div bind:this={host}></div>
	{#if unavailable}
		<p class="text-sm text-red-600">
			The challenge could not be loaded. Reload the page to try again.
		</p>
	{/if}
{/if}
