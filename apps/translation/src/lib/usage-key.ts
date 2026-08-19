import { displayKey } from "./keys";

/** Static spellings used by namespaced and namespace-bound i18next calls. */
export function usageNeedles(
	key: string,
	namespace: string,
	defaultNamespace: string,
): string[] {
	const displayed = displayKey(key);
	return namespace === defaultNamespace
		? [displayed]
		: [`${namespace}:${displayed}`, displayed];
}
