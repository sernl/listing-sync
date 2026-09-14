<script lang="ts">
	import Button from '$lib/Button.svelte';

	let {
		page,
		hasNext,
		busy = false,
		label = 'Pagination',
		summary,
		onprevious,
		onnext
	}: {
		page: number;
		hasNext: boolean;
		busy?: boolean;
		label?: string;
		summary?: string;
		onprevious: () => void;
		onnext: () => void;
	} = $props();
</script>

<nav class="pagination" aria-label={label} aria-busy={busy}>
	<div class="page-summary" role="status" aria-live="polite">
		{#if summary}<span>{summary}</span>{/if}
		<span class="page-number">Page {page}</span>
	</div>
	<div class="page-actions">
		<Button
			disabled={busy || page <= 1}
			reason={busy ? 'Loading this page' : page <= 1 ? 'This is the first page' : undefined}
			onclick={onprevious}>Previous</Button
		>
		<Button
			disabled={busy || !hasNext}
			reason={busy ? 'Loading this page' : !hasNext ? 'This is the last page' : undefined}
			onclick={onnext}>Next</Button
		>
	</div>
</nav>

<style>
	.pagination {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 16px;
		padding-block: 16px;
		border-top: 1px solid var(--line);
	}

	.page-summary {
		display: flex;
		flex-wrap: wrap;
		gap: 6px 16px;
		color: var(--muted);
		font-size: 13px;
	}

	.page-number {
		white-space: nowrap;
	}

	.page-actions {
		display: flex;
		gap: 8px;
		flex-shrink: 0;
	}

	@media (max-width: 620px) {
		.pagination {
			flex-direction: column;
			align-items: stretch;
			gap: 12px;
		}

		.page-summary,
		.page-actions {
			justify-content: space-between;
		}
	}
</style>
