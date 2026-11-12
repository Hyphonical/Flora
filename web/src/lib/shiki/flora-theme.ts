import type { ThemeRegistration } from 'shiki';

export const FLORA_THEME_NAME = 'flora';

/**
 * Flora's own Shiki theme — the only theme registered with the highlighter.
 *
 * Every colour resolves to a CSS custom property declared in `app.css`, so the
 * palette lives in one place and the highlighter follows the app's theme.
 * The values map one-to-one onto the palette in `PLAN.md`.
 */
export const floraTheme: ThemeRegistration = {
	name: FLORA_THEME_NAME,
	type: 'dark',
	bg: 'transparent',
	fg: 'var(--code-fg)',
	settings: [
		{
			scope: ['comment', 'punctuation.definition.comment'],
			settings: { foreground: 'var(--code-comment)' }
		},
		{
			scope: ['string', 'string.quoted', 'string.template', 'punctuation.definition.string'],
			settings: { foreground: 'var(--code-string)' }
		},
		{
			scope: ['keyword', 'storage', 'keyword.control', 'keyword.operator'],
			settings: { foreground: 'var(--code-keyword)' }
		},
		{
			scope: ['constant.numeric', 'constant.language', 'constant.character'],
			settings: { foreground: 'var(--code-number)' }
		},
		{
			scope: ['entity.name.function', 'support.function', 'meta.function-call'],
			settings: { foreground: 'var(--code-func)' }
		},
		{
			scope: ['entity.name.type', 'entity.name.class', 'support.type', 'support.class'],
			settings: { foreground: 'var(--code-type)' }
		},
		{
			scope: ['entity.name.tag', 'meta.tag'],
			settings: { foreground: 'var(--code-tag)' }
		},
		{
			scope: ['variable', 'variable.parameter', 'variable.other'],
			settings: { foreground: 'var(--code-fg)' }
		},
		{
			scope: ['punctuation', 'meta.brace', 'meta.delimiter'],
			settings: { foreground: 'var(--code-punct)' }
		},
		{
			scope: ['markup.heading'],
			settings: { foreground: 'var(--code-heading)', fontStyle: 'bold' }
		},
		{ scope: ['markup.bold'], settings: { fontStyle: 'bold' } },
		{ scope: ['markup.italic'], settings: { fontStyle: 'italic' } },
		{
			scope: ['markup.underline.link', 'markup.link'],
			settings: { foreground: 'var(--code-string)' }
		},
		{
			scope: ['markup.inline.raw', 'markup.raw'],
			settings: { foreground: 'var(--code-fg)' }
		},
		{
			scope: ['markup.list', 'markup.list punctuation'],
			settings: { foreground: 'var(--code-keyword)' }
		},
		{
			scope: ['markup.quote'],
			settings: { foreground: 'var(--code-comment)', fontStyle: 'italic' }
		},
		{
			scope: ['invalid'],
			settings: { foreground: 'var(--code-number)' }
		}
	]
};
