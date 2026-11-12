<script lang="ts">
	import { highlight, highlightSync } from '../shiki/highlighter';

	interface Props {
		/** The editable source. */
		code?: string;
		/** Used to pick the language; the file name is enough. */
		filename?: string;
		/** Height of the editor in pixels. */
		minHeight?: number;
	}

	let { code = $bindable(''), filename = '', minHeight = 380 }: Props = $props();

	let html = $state('');
	let layer = $state<HTMLElement | null>(null);
	let input = $state<HTMLTextAreaElement | null>(null);

	// The file the current highlight belongs to; plain text is shown until it matches.
	let renderedFor = '';

	const lineCount = $derived(Math.max(1, code.split('\n').length));

	$effect(() => {
		const source = code;
		const name = filename;
		let cancelled = false;

		if (name !== renderedFor) {
			renderedFor = name;
			html = '';
		}

		// Once the engine and the grammar are loaded, highlighting is immediate.
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

	/**
	 * The textarea's scrollbar narrows its content box, which would make the
	 * highlight layer wrap at a different width and drift away from the caret.
	 * Reserve the same horizontal space in the layer.
	 */
	function syncMetrics() {
		if (!layer || !input) return;
		const scrollbar = input.offsetWidth - input.clientWidth;
		layer.style.paddingRight = `${16 + scrollbar}px`;
	}

	function syncScroll() {
		if (!layer || !input) return;
		layer.scrollTop = input.scrollTop;
		layer.scrollLeft = input.scrollLeft;
	}

	$effect(() => {
		if (!input) return;
		// Fires on mount and whenever the textarea's content box changes,
		// including when a scrollbar appears or the editor is resized.
		const observer = new ResizeObserver(syncMetrics);
		observer.observe(input);
		return () => observer.disconnect();
	});
</script>

<div class="code-editor" style:--code-min-height="{minHeight}px">
	<div class="gutter mono" aria-hidden="true">
		{#each Array.from({ length: lineCount }, (_, i) => i + 1) as line (line)}
			<div>{line}</div>
		{/each}
	</div>
	<div class="code-surface">
		<pre class="code-layer" bind:this={layer} aria-hidden="true">{@html html}</pre>
		<textarea
			class="code-input"
			class:has-highlight={html !== ''}
			bind:value={code}
			bind:this={input}
			onscroll={syncScroll}
			spellcheck="false"
			aria-label="File contents"
		></textarea>
	</div>
</div>

<style>
	.code-editor {
		display: grid;
		grid-template-columns: 44px minmax(0, 1fr);
		background: #151515;
		min-height: var(--code-min-height, 380px);
	}

	.gutter {
		padding: 14px 8px 14px 0;
		text-align: right;
		color: var(--muted);
		font-size: 12.5px;
		line-height: 1.65;
		background: #131313;
		border-right: 1px solid #222222;
		user-select: none;
	}

	.code-surface {
		position: relative;
		min-width: 0;
	}

	.code-layer,
	.code-input {
		box-sizing: border-box;
		margin: 0;
		border: none;
		padding: 14px 16px;
		font-family: var(--font-mono);
		font-variant-ligatures: none;
		font-size: 12.5px;
		line-height: 1.65;
		tab-size: 2;
		white-space: pre-wrap;
		overflow-wrap: break-word;
	}

	.code-layer {
		position: absolute;
		inset: 0;
		overflow: hidden;
		pointer-events: none;
		background: transparent;
		color: var(--code-fg);
	}

	.code-input {
		position: relative;
		display: block;
		width: 100%;
		min-height: var(--code-min-height, 380px);
		resize: vertical;
		overflow: auto;
		background: transparent;
		outline: none;
		color: var(--code-fg);
		caret-color: var(--code-fg);
	}

	/* Once the layer carries the text, the textarea keeps only the caret. */
	.code-input.has-highlight {
		color: transparent;
	}

	.code-input::selection {
		background: var(--accent-soft);
	}
</style>
