import { deDevelopers } from "./developers-de";
import { enDevelopers } from "./developers-en";
import { esDevelopers } from "./developers-es";
import { frDevelopers } from "./developers-fr";
import { itDevelopers } from "./developers-it";
import { jaDevelopers } from "./developers-ja";
import { koDevelopers } from "./developers-ko";
import { nlDevelopers } from "./developers-nl";
import { ptDevelopers } from "./developers-pt";
import { svDevelopers } from "./developers-sv";
import { zhDevelopers } from "./developers-zh";

export const translationsDevelopers: Record<string, Record<string, string>> = {
	en: enDevelopers,
	de: deDevelopers,
	fr: frDevelopers,
	es: esDevelopers,
	zh: zhDevelopers,
	ja: jaDevelopers,
	ko: koDevelopers,
	pt: ptDevelopers,
	it: itDevelopers,
	nl: nlDevelopers,
	sv: svDevelopers,
};

export function tDevelopers(lang: string, key: string): string {
	return (
		translationsDevelopers[lang]?.[key] ?? translationsDevelopers.en[key] ?? key
	);
}
