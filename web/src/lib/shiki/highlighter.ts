import type * as ShikiBundle from 'shiki/bundle/full';
import type { BundledLanguage, Highlighter } from 'shiki/bundle/full';
import { FLORA_THEME_NAME, floraTheme } from './flora-theme';
import { buildLanguageCatalog, resolveLanguage, type LanguageCatalog } from './languages';

type Bundle = typeof ShikiBundle;

/** Files larger than this are shown as plain text instead of being highlighted. */
const MAX_HIGHLIGHT_CHARS = 512 * 1024;

/** Languages whose fenced code blocks are highlighted with their own grammar. */
const FENCE_LANGUAGES: Record<string, true> = { markdown: true, mdx: true };

const FENCE_PATTERN = /^[ \t]*(?:```|~~~)[ \t]*([\w+#.-]+)/gm;

let bundlePromise: Promise<Bundle> | null = null;
let highlighterPromise: Promise<Highlighter> | null = null;
let catalogPromise: Promise<LanguageCatalog> | null = null;

let readyHighlighter: Highlighter | null = null;
let readyCatalog: LanguageCatalog | null = null;

/**
 * The full Shiki bundle is several megabytes, so it is fetched the first time
 * a file is opened rather than on page load.
 */
function loadBundle(): Promise<Bundle> {
	bundlePromise ??= import('shiki/bundle/full');
	return bundlePromise;
}

function getHighlighter(): Promise<Highlighter> {
	highlighterPromise ??= loadBundle().then((bundle) =>
		bundle.createHighlighter({
			themes: [floraTheme],
			langs: []
		})
	);
	return highlighterPromise;
}

function getCatalog(): Promise<LanguageCatalog> {
	catalogPromise ??= loadBundle().then((bundle) =>
		buildLanguageCatalog(
			bundle.bundledLanguagesInfo,
			bundle.bundledLanguages,
			bundle.bundledLanguagesAlias
		)
	);
	return catalogPromise;
}

/**
 * Collects the languages named by fenced code blocks.
 *
 * A fenced block is only coloured with its own grammar once that grammar is
 * loaded, so Markdown needs its fence languages loaded up front.
 */
function collectFenceLanguages(code: string): string[] {
	const found = new Set<string>();
	for (const match of code.matchAll(FENCE_PATTERN)) {
		const name = match[1]?.toLowerCase();
		if (name) found.add(name);
	}
	return [...found];
}

/**
 * The file's own language, plus the grammars its fenced code blocks need.
 */
function wantedLanguages(code: string, filename: string, catalog: LanguageCatalog): string[] {
	const requested = resolveLanguage(filename, catalog);
	const language = catalog.canonical.get(requested) ?? requested;
	const wanted = new Set<string>([language]);
	if (language in FENCE_LANGUAGES) {
		for (const fence of collectFenceLanguages(code)) {
			const canonical = catalog.canonical.get(fence);
			if (canonical) wanted.add(canonical);
		}
	}
	return [...wanted];
}

/**
 * Highlights `code` immediately, without waiting for anything.
 *
 * Returns `null` when the engine or a required grammar is not loaded yet, in
 * which case the caller should fall back to {@link highlight}.
 */
export function highlightSync(code: string, filename: string): string | null {
	if (!readyHighlighter || !readyCatalog) return null;
	if (code.length > MAX_HIGHLIGHT_CHARS) return null;

	const wanted = wantedLanguages(code, filename, readyCatalog);
	const loaded = readyHighlighter.getLoadedLanguages();
	for (const lang of wanted) {
		if (lang === 'text') continue;
		if (!loaded.includes(lang)) return null;
	}

	return readyHighlighter.codeToHtml(code, {
		lang: wanted[0] ?? 'text',
		theme: FLORA_THEME_NAME,
		structure: 'inline'
	});
}

export interface HighlightResult {
	/** The highlighted spans, ready to be placed inside a `<pre>`. */
	html: string;
	/** The canonical language id, e.g. `markdown`. */
	language: string;
}

/**
 * Loads whatever `filename` needs, then highlights `code` with Flora's theme.
 *
 * Returns empty HTML when the file cannot or should not be highlighted, in
 * which case the caller falls back to plain text.
 */
export async function highlight(code: string, filename: string): Promise<HighlightResult> {
	if (code.length > MAX_HIGHLIGHT_CHARS) {
		return { html: '', language: 'text' };
	}

	const [highlighter, catalog] = await Promise.all([getHighlighter(), getCatalog()]);
	const wanted = wantedLanguages(code, filename, catalog);

	try {
		for (const lang of wanted) {
			if (lang === 'text') continue;
			if (!highlighter.getLoadedLanguages().includes(lang)) {
				await highlighter.loadLanguage(lang as BundledLanguage);
			}
		}
		readyHighlighter = highlighter;
		readyCatalog = catalog;
		const language = wanted[0] ?? 'text';
		const html = highlighter.codeToHtml(code, {
			lang: language,
			theme: FLORA_THEME_NAME,
			structure: 'inline'
		});
		return { html, language };
	} catch (error) {
		console.warn(`could not highlight ${filename}`, error);
		return { html: '', language: 'text' };
	}
}
