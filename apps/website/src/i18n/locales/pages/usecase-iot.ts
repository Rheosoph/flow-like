import { deIot } from "./usecase-iot-de";
import { enIot } from "./usecase-iot-en";
import { esIot } from "./usecase-iot-es";
import { frIot } from "./usecase-iot-fr";
import { itIot } from "./usecase-iot-it";
import { jaIot } from "./usecase-iot-ja";
import { koIot } from "./usecase-iot-ko";
import { nlIot } from "./usecase-iot-nl";
import { ptIot } from "./usecase-iot-pt";
import { svIot } from "./usecase-iot-sv";
import { zhIot } from "./usecase-iot-zh";

export const translationsIot: Record<string, Record<string, string>> = {
	en: enIot,
	de: deIot,
	fr: frIot,
	es: esIot,
	zh: zhIot,
	ja: jaIot,
	ko: koIot,
	pt: ptIot,
	it: itIot,
	nl: nlIot,
	sv: svIot,
};

export function tIot(lang: string, key: string): string {
	return translationsIot[lang]?.[key] ?? translationsIot.en[key] ?? key;
}
