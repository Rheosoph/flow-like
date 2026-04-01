import { deOffice } from "./industry-office-de";
import { enOffice } from "./industry-office-en";
import { esOffice } from "./industry-office-es";
import { frOffice } from "./industry-office-fr";
import { itOffice } from "./industry-office-it";
import { jaOffice } from "./industry-office-ja";
import { koOffice } from "./industry-office-ko";
import { nlOffice } from "./industry-office-nl";
import { ptOffice } from "./industry-office-pt";
import { svOffice } from "./industry-office-sv";
import { zhOffice } from "./industry-office-zh";

export const translationsOffice: Record<string, Record<string, string>> = {
	en: enOffice,
	de: deOffice,
	fr: frOffice,
	es: esOffice,
	zh: zhOffice,
	ja: jaOffice,
	ko: koOffice,
	pt: ptOffice,
	it: itOffice,
	nl: nlOffice,
	sv: svOffice,
};

export function tOffice(lang: string, key: string): string {
	return translationsOffice[lang]?.[key] ?? translationsOffice.en[key] ?? key;
}
