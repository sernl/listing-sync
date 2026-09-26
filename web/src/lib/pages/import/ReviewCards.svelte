<script lang="ts">
	// The duplicate review, as one panel of cards. Rendered by the marketplace
	// run's page and by the spreadsheet batch's page alike, because the
	// question is the same question whichever way the resource came in: two
	// listings, one sentence of evidence, and three answers.
	//
	// No score is shown anywhere here, and none is on the wire. The sentence
	// is the server's own, generated from the layer it stored; a seller who
	// reads "94%" starts arguing with the number instead of looking at the
	// two listings.

	import Button from '$lib/Button.svelte';
	import Explain from '$lib/Explain.svelte';
	import MarketplaceMark from '$lib/MarketplaceMark.svelte';
	import Pagination from '$lib/Pagination.svelte';
	import StatusPill from '$lib/StatusPill.svelte';
	import '$lib/flow.css';
	import './run.css';
	import type { PairFieldChoice, PairSide, ReviewPairView } from '$lib/api';
	import {
		MERGE_FIELDS,
		MERGE_IS_REVERSIBLE,
		REVIEW_DIFFERENT,
		REVIEW_LATER,
		REVIEW_SAME,
		WHICH_SIDE_WINS,
		pageCount,
		pageSummary,
		reviewCard
	} from './run-view';

	let {
		pairs,
		onsame,
		ondifferent,
		onlater
	}: {
		pairs: readonly ReviewPairView[];
		/** The seller merged the pair: which resource survives, and whose
		 *  wording it keeps per field. */
		onsame: (lo: string, hi: string, keep: string, fields: PairFieldChoice) => void;
		ondifferent: (lo: string, hi: string) => void;
		onlater: (lo: string, hi: string) => void;
	} = $props();

	/** How many questions are put at once.
	 *
	 *  Each card is two listings, two covers and three answers, so a page of
	 *  them is a page of real reading. Ten is what a seller can work through
	 *  before the list stops being a queue and starts being a wall. */
	const PER_PAGE = 10;

	let page = $state(1);

	const all = $derived(pairs.map(reviewCard));
	const pages = $derived(pageCount(all.length, PER_PAGE));
	// Answering a card removes it from the list the server sends, so a page
	// that empties itself would strand the seller on a page past the end.
	const at = $derived(Math.min(page, pages));
	const cards = $derived(all.slice((at - 1) * PER_PAGE, at * PER_PAGE));

	// Which card is in its which-side-wins step, by pair key. A card is one
	// question at a time: answering "same" opens the field choices under that
	// card rather than replacing the page, so the two listings the seller is
	// comparing stay on screen while they choose between them.
	let merging = $state<string | null>(null);
	let keeping = $state<PairSide>('lo');
	let fields = $state<PairFieldChoice>({});

	function key(lo: string, hi: string): string {
		return `${lo}:${hi}`;
	}

	/** "Keep this one": the pair is one resource and this side survives. The
	 *  field choices open under the card, defaulting to the kept side. */
	function openMerge(lo: string, hi: string, side: PairSide) {
		merging = key(lo, hi);
		keeping = side;
		fields = {};
	}

	function chooseField(field: 'title' | 'description' | 'price', side: PairSide) {
		fields = { ...fields, [field]: side };
	}

	/** The resource that survives.
	 *
	 *  A side that is still an unimported item has no resource identifier of
	 *  its own, so the pair's own handle for that side is what is sent: the
	 *  server is what knows how to resolve it, and inventing one here would
	 *  name a resource that does not exist. */
	function survivor(lo: string, hi: string): string {
		return keeping === 'lo' ? lo : hi;
	}
</script>

<!-- The duplicate questions, as a list the parent's step titles. Each card
     puts the two listings side by side with one decision under each. -->
<div class="rv-list">
	{#each cards as card (key(card.lo, card.hi))}
		{@const open = merging === key(card.lo, card.hi)}
		<article class="rv-card">
			<p class="rv-say">{card.sentence}</p>

			<div class="rv-sides">
				{#each card.sides as side, index (side.side)}
					{#if index === 1}<span class="rv-or" aria-hidden="true">or</span>{/if}
					<div class="rv-side" class:win={open && keeping === side.side}>
						<div class="rv-head">
							{#if side.marketplace !== null}
								<MarketplaceMark marketplace={side.marketplace} size={18} />
							{/if}
							<StatusPill tone={side.product === null ? 'run' : 'flat'} label={side.standing} />
						</div>
						{#if side.coverUrl !== null}
							<img class="rv-cover" src={side.coverUrl} alt="" width="120" height="90" />
						{:else}
							<div class="rv-cover rv-none" aria-hidden="true"></div>
						{/if}
						<p class="rv-title res-name">{side.title}</p>
						<p class="rv-facts">
							<span class="price">{side.price}</span>
							{#if side.grades !== ''}<span>{side.grades}</span>{/if}
						</p>
						{#if !open}
							<Button
								small
								tier="primary"
								icon="check"
								onclick={() => openMerge(card.lo, card.hi, side.side)}
							>
								{REVIEW_SAME}
							</Button>
						{:else if keeping === side.side}
							<StatusPill tone="ok" label="Keeping this one" />
						{/if}
					</div>
				{/each}
			</div>

			{#if open}
				<div class="rv-merge">
					<details class="flow-more">
						<summary>Mix the wording</summary>
						<p class="rv-say">{WHICH_SIDE_WINS}</p>
						{#each MERGE_FIELDS as field (field.field)}
							<fieldset class="rv-field">
								<legend>{field.label}</legend>
								{#each card.sides as side (side.side)}
									<label>
										<input
											type="radio"
											name={`${field.field}-${key(card.lo, card.hi)}`}
											checked={(fields[field.field] ?? keeping) === side.side}
											onchange={() => chooseField(field.field, side.side)}
										/>
										{side.title}
									</label>
								{/each}
							</fieldset>
						{/each}
					</details>

					<div class="flow-actions">
						<Button
							tier="primary"
							icon="check"
							onclick={() => onsame(card.lo, card.hi, survivor(card.lo, card.hi), fields)}
						>
							Merge them
						</Button>
						<Button tier="quiet" onclick={() => (merging = null)}>Cancel</Button>
						<Explain title="Undoing a merge" label="">
							<p>{MERGE_IS_REVERSIBLE}</p>
						</Explain>
					</div>
				</div>
			{:else}
				<div class="flow-actions rv-rest">
					<Button small onclick={() => ondifferent(card.lo, card.hi)}>{REVIEW_DIFFERENT}</Button>
					<Button small tier="quiet" onclick={() => onlater(card.lo, card.hi)}>{REVIEW_LATER}</Button>
				</div>
			{/if}
		</article>
	{/each}
	{#if all.length > PER_PAGE}
		<Pagination
			page={at}
			hasNext={at < pages}
			label="Duplicate questions"
			summary={`${pageSummary((at - 1) * PER_PAGE, cards.length, all.length, 'questions')} · Page ${at} of ${pages}`}
			onprevious={() => (page = at - 1)}
			onnext={() => (page = at + 1)}
		/>
	{/if}
</div>
