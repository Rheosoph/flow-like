import { installWorkbenchDom } from "../testing/dom";

/**
 * DOM globals for the stage's tests: the workbench harness plus what Plate's static editors reach for
 * (`Range`, `Selection`, `DOMParser`, …; memory happy-dom-react-dom-import-order). Call before importing components.
 */
const EDITOR_GLOBALS = [
	"Document",
	"ShadowRoot",
	"Range",
	"Selection",
	"Comment",
	"DOMParser",
] as const;

export function installStageDom() {
	const dom = installWorkbenchDom();
	const win = dom.window as unknown as Record<string, unknown>;
	const saved = EDITOR_GLOBALS.map(
		(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
	);
	for (const key of EDITOR_GLOBALS)
		Object.defineProperty(globalThis, key, {
			configurable: true,
			writable: true,
			value: win[key],
		});
	const selection = win.getSelection as (() => unknown) | undefined;
	Object.defineProperty(globalThis, "getSelection", {
		configurable: true,
		writable: true,
		value: selection?.bind(win),
	});
	return {
		...dom,
		restore() {
			for (const [key, descriptor] of saved) {
				if (descriptor) Object.defineProperty(globalThis, key, descriptor);
				else delete (globalThis as Record<string, unknown>)[key];
			}
			(globalThis as Record<string, unknown>).getSelection = undefined;
			dom.restore();
		},
	};
}
