<script lang="ts">
	import { dayLabel, formatCount, lineGeometry, shortDay, type DayPoint } from './site-analytics';

	let { days, label }: { days: readonly DayPoint[]; label: string } = $props();

	// Plain pixels, as `MetricBars` draws: the SVG has no viewBox, so its
	// units are CSS pixels and the axis text stays legible at 390px while the
	// line stretches to whatever width the panel has.
	const HEIGHT = 220;
	const FRAME = { left: 36, right: 12, top: 12, bottom: 28 };

	let width = $state(0);

	const geometry = $derived(lineGeometry(days, { ...FRAME, width, height: HEIGHT }));
	// A dot per day reads at 7 and turns into a smear at 90.
	const dotted = $derived(days.length <= 31);
</script>

<div class="sa-line-box" bind:clientWidth={width}>
	{#if width > 0}
		<svg class="sa-line" height={HEIGHT} {width} role="img" aria-label={label}>
			{#each geometry.ticks as tick (tick.value)}
				<line class="sa-grid" x1={FRAME.left} x2={width - FRAME.right} y1={tick.y} y2={tick.y} />
				<text class="sa-tick" x={FRAME.left - 8} y={tick.y + 4} text-anchor="end">
					{formatCount(tick.value)}
				</text>
			{/each}
			<path class="sa-area" d={geometry.area} />
			<path class="sa-stroke" d={geometry.line} />
			{#each geometry.points as point (point.day)}
				<g>
					<title>
						{dayLabel(point.day)}: {formatCount(point.visitors)} visitors, {formatCount(
							point.pageviews
						)} page views
					</title>
					<circle
						class={dotted ? 'sa-dot' : 'sa-hit'}
						cx={point.x}
						cy={point.y}
						r={dotted ? 3.5 : 6}
					/>
				</g>
			{/each}
			{#each geometry.labels as tick (tick.day)}
				<text
					class="sa-day"
					x={tick.x}
					y={HEIGHT - 8}
					text-anchor={tick.x > width - FRAME.right - 24 ? 'end' : 'middle'}
					>{shortDay(tick.day)}</text
				>
			{/each}
		</svg>
	{/if}
</div>
