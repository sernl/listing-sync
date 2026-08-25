// Fixed-row-height windowing for the product table: which slice of rows is
// visible, with overscan, and the padding that keeps the scrollbar honest.
// Fifty lines instead of a virtualisation dependency, at founder scale.

export interface Window {
	start: number;
	end: number;
	padTop: number;
	padBottom: number;
}

export function visibleWindow(
	total: number,
	rowHeight: number,
	scrollTop: number,
	viewportHeight: number,
	overscan = 5
): Window {
	if (total === 0 || rowHeight <= 0) {
		return { start: 0, end: 0, padTop: 0, padBottom: 0 };
	}
	const first = Math.floor(scrollTop / rowHeight);
	const visible = Math.ceil(viewportHeight / rowHeight);
	const start = Math.max(0, first - overscan);
	const end = Math.min(total, first + visible + overscan);
	return {
		start,
		end,
		padTop: start * rowHeight,
		padBottom: (total - end) * rowHeight
	};
}
