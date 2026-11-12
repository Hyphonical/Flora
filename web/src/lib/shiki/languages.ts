/**
 * Language resolution for the highlighter.
 *
 * Shiki's full bundle ships every grammar and every alias, so there is no
 * hand-maintained extension table here: the catalogue is derived from the
 * bundle's own metadata at runtime.
 */

export interface LanguageCatalog {
	/** Every name Shiki accepts, canonical ids and aliases alike (`md`, `rs`, `ps1`, …). */
	known: Set<string>;
	/** Maps an accepted name onto its canonical language id (`md` → `markdown`). */
	canonical: Map<string, string>;
}

interface LanguageInfo {
	id: string;
	import?: unknown;
}

/**
 * Builds the catalogue from the bundle metadata.
 *
 * Canonical ids come from `bundledLanguagesInfo`; aliases are the extra keys
 * in `bundledLanguages` and `bundledLanguagesAlias`. Each alias points at the
 * same grammar registration as its canonical id, which is how aliases are
 * matched back to their language.
 */
export function buildLanguageCatalog(
	info: readonly LanguageInfo[],
	languages: Record<string, unknown>,
	aliases: Record<string, unknown>
): LanguageCatalog {
	const byRegistration = new Map<unknown, string>();
	for (const entry of info) {
		if (entry.import !== undefined) {
			byRegistration.set(entry.import, entry.id);
		}
	}

	const known = new Set<string>([...Object.keys(languages), ...Object.keys(aliases)]);
	const canonical = new Map<string, string>();
	for (const name of known) {
		const registration = languages[name] ?? aliases[name];
		canonical.set(name, byRegistration.get(registration) ?? name);
	}

	return { known, canonical };
}

/**
 * Picks the Shiki language for a filename.
 *
 * The extension is used directly (`deploy-checklist.md` → `md`), which Shiki
 * resolves to the `markdown` grammar. Extensionless names work too, because
 * `Dockerfile` and `Makefile` are registered aliases.
 */
export function resolveLanguage(filename: string, catalog: LanguageCatalog): string {
	const base = filename.split(/[/\\]/).pop() ?? filename;
	const dot = base.lastIndexOf('.');
	const candidate = (dot > 0 ? base.slice(dot + 1) : base).toLowerCase();
	return catalog.known.has(candidate) ? candidate : 'text';
}
