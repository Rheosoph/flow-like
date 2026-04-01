import { deProcessAutomation } from "./usecase-process-automation-de";
import { enProcessAutomation } from "./usecase-process-automation-en";
import { esProcessAutomation } from "./usecase-process-automation-es";
import { frProcessAutomation } from "./usecase-process-automation-fr";
import { itProcessAutomation } from "./usecase-process-automation-it";
import { jaProcessAutomation } from "./usecase-process-automation-ja";
import { koProcessAutomation } from "./usecase-process-automation-ko";
import { nlProcessAutomation } from "./usecase-process-automation-nl";
import { ptProcessAutomation } from "./usecase-process-automation-pt";
import { svProcessAutomation } from "./usecase-process-automation-sv";
import { zhProcessAutomation } from "./usecase-process-automation-zh";

export const translationsProcessAutomation: Record<string, Record<string, string>> = {
	en: enProcessAutomation,
	de: deProcessAutomation,
	fr: frProcessAutomation,
	es: esProcessAutomation,
	zh: zhProcessAutomation,
	ja: jaProcessAutomation,
	ko: koProcessAutomation,
	pt: ptProcessAutomation,
	it: itProcessAutomation,
	nl: nlProcessAutomation,
	sv: svProcessAutomation,
};

export function tProcessAutomation(lang: string, key: string): string {
	return translationsProcessAutomation[lang]?.[key] ?? translationsProcessAutomation.en[key] ?? key;
}
