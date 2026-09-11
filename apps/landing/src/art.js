/**
 * Geometry for the drawn compositions, so a box and the mark inside it are
 * arithmetic in one place rather than four sets of hand-typed coordinates.
 */

/**
 * Where a mark sits inside a box it is centred in.
 *
 * A wordmark is the name drawn across a wide canvas and an icon is square, so
 * the slot differs by shape rather than a drawing being squashed into one box.
 * Both are capped by the box, which is what lets the same function place a
 * mark in a hero tile and in a listing sheet's chip.
 */
export const markSlot = (mark, x, y, w, h) => {
	if (mark.shape === 'wordmark') {
		const width = Math.min(w - 24, 80);
		return { x: x + (w - width) / 2, y: y + h / 2 - 10, width, height: 20 };
	}
	const size = Math.min(34, h - 10);
	return { x: x + w / 2 - size / 2, y: y + h / 2 - size / 2, width: size, height: size };
};
