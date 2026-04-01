import { deGovDefense } from "./industry-gov-defense-de";
import { enGovDefense } from "./industry-gov-defense-en";
import { esGovDefense } from "./industry-gov-defense-es";
import { frGovDefense } from "./industry-gov-defense-fr";
import { itGovDefense } from "./industry-gov-defense-it";
import { jaGovDefense } from "./industry-gov-defense-ja";
import { koGovDefense } from "./industry-gov-defense-ko";
import { nlGovDefense } from "./industry-gov-defense-nl";
import { ptGovDefense } from "./industry-gov-defense-pt";
import { svGovDefense } from "./industry-gov-defense-sv";
import { zhGovDefense } from "./industry-gov-defense-zh";

export const translationsGovDefense: Record<string, Record<string, string>> = {
	en: enGovDefense,
	de: deGovDefense,
	fr: frGovDefense,
	es: esGovDefense,
	zh: zhGovDefense,
	ja: jaGovDefense,
	ko: koGovDefense,
	pt: ptGovDefense,
	it: itGovDefense,
	nl: nlGovDefense,
	sv: svGovDefense,
};

export function tGovDefense(lang: string, key: string): string {
	return (
		translationsGovDefense[lang]?.[key] ?? translationsGovDefense.en[key] ?? key
	);
}
