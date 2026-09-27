<script lang="ts">
	import type { CopyFormat } from '$lib/generated/vocab';
	import { markdownToHtml, sanitiseHtml, tesText } from '$lib/rich-text';

	// The description as one marketplace will carry it, read-only: TPT's field
	// takes HTML, so it is drawn as formatting with the markup one press away;
	// Tes takes plain text, so it is shown as the text that will be posted.
	let {
		body,
		format,
		inventory,
		labelledby
	}: {
		body: string;
		format: CopyFormat;
		inventory: 'Tpt' | 'Tes';
		labelledby: string;
	} = $props();

	/** What TPT receives. A Markdown body is rendered as the projection
	 *  renders it; either way only the allow-list is drawn here, so an imported
	 *  body's markup cannot run in the console. */
	const html = $derived(
		inventory === 'Tpt' ? (format === 'Html' ? sanitiseHtml(body) : markdownToHtml(body).html) : ''
	);
	const text = $derived(inventory === 'Tes' ? tesText(body, format) : '');
</script>

{#if inventory === 'Tpt'}
	<div class="res-preview" aria-labelledby={labelledby} role="document">
		<!-- Allow-listed above, which is what makes `{@html}` safe here: the
		     eight elements, and no attribute but a web or mail href. -->
		{@html html}
	</div>
	<details class="res-preview-source">
		<summary>See the HTML TPT gets</summary>
		<code>{html}</code>
	</details>
{:else}
	<div class="res-preview plain" aria-labelledby={labelledby} role="document">{text}</div>
{/if}
