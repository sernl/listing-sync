<script lang="ts">
	import { ICONS, type IconName } from '$lib/icons';

	let { name, size = 18 }: { name: IconName; size?: number } = $props();

	const Glyph = $derived(ICONS[name]);
</script>

<!-- Always decorative: every glyph in this console sits inside an element that
     already carries the name, so a second one here would be read twice. A glyph
     that ever needs its own name needs `role="img"` beside the label, because a
     bare `aria-label` on `<svg>` is not reliably exposed. -->
<Glyph {size} aria-hidden="true" />
