import { deSecurity } from "./security-de";
import { enSecurity } from "./security-en";
import { esSecurity } from "./security-es";
import { frSecurity } from "./security-fr";
import { itSecurity } from "./security-it";
import { jaSecurity } from "./security-ja";
import { koSecurity } from "./security-ko";
import { nlSecurity } from "./security-nl";
import { ptSecurity } from "./security-pt";
import { svSecurity } from "./security-sv";
import { zhSecurity } from "./security-zh";

export const translationsSecurity: Record<string, Record<string, string>> = {
	en: enSecurity,
	de: deSecurity,
	es: esSecurity,
	fr: frSecurity,
	zh: zhSecurity,
	ja: jaSecurity,
	ko: koSecurity,
	pt: ptSecurity,
	it: itSecurity,
	nl: nlSecurity,
	sv: svSecurity,
};

export function tSecurity(lang: string, key: string): string {
	return (
		translationsSecurity[lang]?.[key] ?? translationsSecurity.en[key] ?? key
	);
}
