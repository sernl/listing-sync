// A modal dialog closes when the seller presses outside it, as well as on
// Escape, which the browser already gives a modal <dialog>.
//
// A press on the backdrop of a modal dialog lands on the dialog element
// itself, because the backdrop is its pseudo-element; a press on anything the
// dialog holds lands on that child. So "outside" is a press whose target is the
// dialog. Every dialog in the console has `padding: 0` and a body filling it,
// which is what keeps that test from also matching the dialog's own edge.
//
// The press has to start and end there. A seller selecting text in a field who
// lets go past the dialog's edge raises a `click` on the dialog, and closing on
// that would throw away what they were typing.
//
// Closing goes through a cancelable `cancel` event, the one Escape raises, so a
// dialog that refuses Escape while it is saving refuses the press too, with the
// same handler. `requestClose()` would do this, but WebKitGTK — the desktop
// app's webview — does not have it.

/** Close `node` on a press outside it, unless its `cancel` handler refuses. */
export function lightDismiss(node: HTMLDialogElement) {
	let startedOutside = false;
	const pressed = (event: Event) => {
		startedOutside = event.target === node;
	};
	const clicked = (event: Event) => {
		const outside = startedOutside && event.target === node;
		startedOutside = false;
		if (!outside || !node.open) {
			return;
		}
		if (node.dispatchEvent(new Event('cancel', { cancelable: true }))) {
			node.close();
		}
	};
	node.addEventListener('pointerdown', pressed);
	node.addEventListener('click', clicked);
	return {
		destroy() {
			node.removeEventListener('pointerdown', pressed);
			node.removeEventListener('click', clicked);
		}
	};
}
