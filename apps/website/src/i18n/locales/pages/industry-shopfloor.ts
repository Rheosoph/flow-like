import { deShopfloor } from "./industry-shopfloor-de";
import { enShopfloor } from "./industry-shopfloor-en";
import { esShopfloor } from "./industry-shopfloor-es";
import { frShopfloor } from "./industry-shopfloor-fr";
import { itShopfloor } from "./industry-shopfloor-it";
import { jaShopfloor } from "./industry-shopfloor-ja";
import { koShopfloor } from "./industry-shopfloor-ko";
import { nlShopfloor } from "./industry-shopfloor-nl";
import { ptShopfloor } from "./industry-shopfloor-pt";
import { svShopfloor } from "./industry-shopfloor-sv";
import { zhShopfloor } from "./industry-shopfloor-zh";

export const translationsShopfloor: Record<string, Record<string, string>> = {
	en: enShopfloor,
	de: deShopfloor,
	fr: frShopfloor,
	es: esShopfloor,
	zh: zhShopfloor,
	ja: jaShopfloor,
	ko: koShopfloor,
	pt: ptShopfloor,
	it: itShopfloor,
	nl: nlShopfloor,
	sv: svShopfloor,
};

export function tShopfloor(lang: string, key: string): string {
	return (
		translationsShopfloor[lang]?.[key] ?? translationsShopfloor.en[key] ?? key
	);
}
