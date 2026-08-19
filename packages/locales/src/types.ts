import type { SourceResources } from "./resources";

/**
 * Namespace names are typed; individual keys are not.
 *
 * Typing every key gives `t()` autocomplete, but the union is one literal type
 * per string — at ~7k keys that pushed a `tsc --noEmit` over packages/ui from
 * roughly two minutes to over fifteen, which is not a trade worth making on
 * every check and every editor keystroke.
 *
 * Missing and stale keys are caught instead by tooling that is built for it:
 * `mise run i18n:status` reports them per locale, `mise run i18n:extract`
 * prunes call-siteless keys, and the studio shows them per namespace. Every
 * instrumented call also passes its English text as a default, so a key that
 * goes missing renders the source string rather than a raw key.
 */
declare module "i18next" {
	interface CustomTypeOptions {
		defaultNS: "common";
		resources: Record<keyof SourceResources, Record<string, string>>;
		returnNull: false;
	}
}
