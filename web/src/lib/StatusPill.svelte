<script lang="ts" module>
	/** The tones this console expresses state in. `soon` is both "checking" on
	 *  a connection and "coming soon" on a download: the two share a grey
	 *  because neither is anything to act on, and they never appear on the same
	 *  card. `flat` is not a state at all — it carries the transport class D1
	 *  requires on every marketplace row. */
	export type Tone = 'ok' | 'warn' | 'bad' | 'run' | 'soon' | 'flat';
</script>

<script lang="ts">
	let { tone = 'soon', label }: { tone?: Tone; label: string } = $props();
</script>

<span class="status {tone}"><i aria-hidden="true"></i>{label}</span>
