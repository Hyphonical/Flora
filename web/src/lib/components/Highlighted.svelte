<script lang="ts">
	import { highlight, highlightSync } from '../shiki/highlighter';

	interface Props {
		/** The source to display. */
		code?: string;
		/** Used to pick the language; the file name is enough. */
		filename?: string;
		/** Shown when there is nothing to display. */
		placeholder?: string;
	}

	let { code = '', filename = '', placeholder = '(empty file)' }: Props = $props();

	let html = $state('');

	$effect(() => {
		const source = code;
		const name = filename;
		let cancelled = false;

		const immediate = highlightSync(source, name);
		if (immediate !== null) {
			html = immediate;
			return;
		}

		highlight(source, name).then((result) => {
			if (cancelled) return;
			html = highlightSync(code, filename) ?? result.html;
		});

		return () => {
			cancelled = true;
		};
	});
</script>

{#if html}
	<pre class="code-highlighted">{@html html}</pre>
{:else}
	<pre class="code-highlighted plain">{code || placeholder}</pre>
{/if}

<style>
	.code-highlighted {
		margin: 0;
		padding: 12px 14px;
		width: 100%;
		align-self: flex-start;
		background: transparent;
		color: var(--code-fg);
		font-family: var(--font-mono);
		font-variant-ligatures: none;
		font-size: 11.5px;
		line-height: 1.55;
		white-space: pre-wrap;
		overflow-wrap: break-word;
		max-height: 205px;
		overflow: hidden;
		tab-size: 2;
	}
</style>
