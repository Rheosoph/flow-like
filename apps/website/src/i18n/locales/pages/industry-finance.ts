import { deFinance } from "./industry-finance-de";
import { enFinance } from "./industry-finance-en";
import { esFinance } from "./industry-finance-es";
import { frFinance } from "./industry-finance-fr";
import { itFinance } from "./industry-finance-it";
import { jaFinance } from "./industry-finance-ja";
import { koFinance } from "./industry-finance-ko";
import { nlFinance } from "./industry-finance-nl";
import { ptFinance } from "./industry-finance-pt";
import { svFinance } from "./industry-finance-sv";
import { zhFinance } from "./industry-finance-zh";

export const translationsFinance: Record<string, Record<string, string>> = {
	en: enFinance,
	de: deFinance,
	fr: frFinance,
	es: esFinance,
	zh: zhFinance,
	ja: jaFinance,
	ko: koFinance,
	pt: ptFinance,
	it: itFinance,
	nl: nlFinance,
	sv: svFinance,
};

export function tFinance(lang: string, key: string): string {
	return translationsFinance[lang]?.[key] ?? translationsFinance.en[key] ?? key;
}
