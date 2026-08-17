import { LANGUAGES, SOURCE_LANGUAGE } from "./config";

export interface LanguageInfo {
	/** BCP-47 code as it appears on disk, e.g. `de`. */
	readonly code: string;
	/** Name in the viewer's own language, e.g. "German" for an English UI. */
	readonly name: string;
	/** Name in the language itself, e.g. "Deutsch". */
	readonly nativeName: string;
	readonly rtl: boolean;
	readonly isSource: boolean;
}

/**
 * Right-to-left scripts. `Intl.Locale.prototype.getTextInfo` would answer this
 * for us but is still missing in enough engines that a lookup is the reliable
 * path — the list is short and stable.
 */
const RTL_LANGUAGES = new Set([
	"ar",
	"dv",
	"fa",
	"he",
	"ku",
	"ps",
	"sd",
	"ur",
	"yi",
]);

function displayName(code: string, locale: string): string {
	try {
		return (
			new Intl.DisplayNames([locale], { type: "language" }).of(code) ?? code
		);
	} catch {
		return code;
	}
}

export function isRtl(code: string): boolean {
	return RTL_LANGUAGES.has(code.split("-")[0].toLowerCase());
}

export function describeLanguage(
	code: string,
	displayLocale = SOURCE_LANGUAGE,
): LanguageInfo {
	return {
		code,
		name: displayName(code, displayLocale),
		nativeName: displayName(code, code),
		rtl: isRtl(code),
		isSource: code === SOURCE_LANGUAGE,
	};
}

/** Every language that has resources on disk, source language first. */
export function listLanguages(displayLocale = SOURCE_LANGUAGE): LanguageInfo[] {
	return [...LANGUAGES]
		.sort((a, b) =>
			a === SOURCE_LANGUAGE
				? -1
				: b === SOURCE_LANGUAGE
					? 1
					: a.localeCompare(b),
		)
		.map((code) => describeLanguage(code, displayLocale));
}
