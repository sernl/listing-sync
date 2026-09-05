<script lang="ts">
	import { clip, type Bar } from './model';

	let {
		bars,
		label,
		format,
		counted = false
	}: {
		bars: readonly Bar[];
		/** What the drawing shows, for a reader who cannot see it. */
		label: string;
		format: (value: number) => string;
		counted?: boolean;
	} = $props();

	// Plain pixels rather than a coordinate space: without a viewBox the SVG's
	// user units are CSS pixels, so a label stays legible at 390px while the
	// bars stay laid out in percentages of whatever width the panel has.
	const ROW = 40;
	const BASELINE = 13;
	const TRACK_TOP = 21;
	const TRACK_H = 8;
	/** Clear space between the end of a label and the start of its value. */
	const GUTTER = 12;

	let width = $state(0);
	let probe = $state<SVGTextElement | null>(null);

	const height = $derived(bars.length * ROW);

	// SVG text neither wraps nor ellipsises, and both the label and its value
	// sit on one baseline, so a label longer than the space left over runs
	// underneath the value. Measuring the real glyph widths is the only way to
	// decide where to cut: a character count cannot tell thirty capital Ws from
	// thirty lower-case ls, and at 1280 it clips a title that would have fit.
	function measurer(): ((text: string) => number) | null {
		if (probe === null || width <= 0) {
			return null;
		}
		const style = getComputedStyle(probe);
		const canvas = document.createElement('canvas');
		const context = canvas.getContext('2d');
		if (context === null) {
			return null;
		}
		context.font = `${style.fontStyle} ${style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
		return (text: string) => context.measureText(text).width;
	}

	function fit(text: string, room: number, measure: (text: string) => number): string {
		if (room <= 0 || measure(text) <= room) {
			return text;
		}
		let low = 0;
		let high = text.length;
		while (low < high) {
			const mid = Math.ceil((low + high) / 2);
			if (measure(`${text.slice(0, mid).trimEnd()}…`) <= room) {
				low = mid;
			} else {
				high = mid - 1;
			}
		}
		return low === 0 ? '…' : `${text.slice(0, low).trimEnd()}…`;
	}

	const drawn = $derived.by(() => {
		const measure = measurer();
		const values = bars.map((bar) => format(bar.value));
		if (measure === null) {
			return bars.map((bar, index) => ({ ...bar, text: clip(bar.label), value: values[index] }));
		}
		const widest = values.reduce((most, value) => Math.max(most, measure(value)), 0);
		const room = width - widest - GUTTER;
		return bars.map((bar, index) => ({
			...bar,
			text: fit(bar.label, room, measure),
			value: values[index]
		}));
	});
</script>

<div class="an-bars-box" bind:clientWidth={width}>
	<svg class="an-bars {counted ? 'an-counted' : ''}" {height} role="img" aria-label={label}>
		<!-- Measured against a real text node in this SVG rather than a font
		     string assembled by hand, so the metric matches what is drawn even
		     if the sheet's family or size changes. -->
		<text class="an-bar-label" bind:this={probe} x="0" y="-100" aria-hidden="true">M</text>
		{#each drawn as bar, index (bar.label + index)}
			<g>
				<title>{bar.title} — {bar.value}</title>
				<text class="an-bar-label" x="0" y={index * ROW + BASELINE}>{bar.text}</text>
				<text class="an-bar-value" x="100%" y={index * ROW + BASELINE} text-anchor="end">
					{bar.value}
				</text>
				<rect
					class="an-bar-track"
					x="0"
					y={index * ROW + TRACK_TOP}
					width="100%"
					height={TRACK_H}
					rx={TRACK_H / 2}
				/>
				{#if bar.share > 0}
					<rect
						class="an-bar-fill"
						x="0"
						y={index * ROW + TRACK_TOP}
						width="{bar.share * 100}%"
						height={TRACK_H}
						rx={TRACK_H / 2}
					/>
				{/if}
			</g>
		{/each}
	</svg>
</div>
