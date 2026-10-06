import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { act } from "react";
import type { FormSessionState, WorkbenchLayout } from "../contracts";
import { byRole, installWorkbenchDom, mountWorkbench } from "../testing/dom";
import { DESKTOP_LAYOUT, PHONE_LAYOUT } from "../testing/layouts";

const dom = installWorkbenchDom();
const { LiveField, liveStore, patchField } = await import("./field-testing");

afterEach(dom.cleanup);
afterAll(dom.restore);

/** happy-dom has no layout: the box reports the content height its own padding classes would give a browser. */
function measureLikeABrowser(box: HTMLElement, desktop: number, touch: number) {
	Object.defineProperty(box, "scrollHeight", {
		configurable: true,
		get: () => (box.className.includes("py-2.75") ? touch : desktop),
	});
}

async function mount(change?: (state: FormSessionState) => FormSessionState) {
	const store = liveStore("idle", { change });
	const field = (layout: WorkbenchLayout) => (
		<LiveField store={store} name="vendor_name" overrides={{ layout }} />
	);
	const view = await mountWorkbench(field(DESKTOP_LAYOUT), {
		layout: DESKTOP_LAYOUT,
	});
	const flip = (layout: WorkbenchLayout) => view.rerender(field(layout));
	return { flip, unmount: view.unmount };
}

/** The document's font set as a browser has it while a web font is still loading: `loadingdone` comes later. */
function loadingFonts() {
	const listeners = new Set<() => void>();
	const fonts = {
		ready: Promise.resolve(),
		addEventListener: (type: string, listener: () => void) => {
			if (type === "loadingdone") listeners.add(listener);
		},
		removeEventListener: (_type: string, listener: () => void) => {
			listeners.delete(listener);
		},
	};
	Object.defineProperty(document, "fonts", {
		configurable: true,
		value: fonts,
	});
	return {
		listening: () => listeners.size,
		arrive: () =>
			act(async () => {
				for (const listener of listeners) listener();
			}),
		restore: () => {
			Reflect.deleteProperty(document, "fonts");
		},
	};
}

describe("a growing box follows its control metrics, not only its text", () => {
	test("the text box refits when the pointer flips to touch and back", async () => {
		const { flip } = await mount();
		const box = byRole("textbox", "Vendor") as HTMLTextAreaElement;
		measureLikeABrowser(box, 34, 42);
		await flip(PHONE_LAYOUT);
		expect(byRole("textbox", "Vendor")).toBe(box);
		expect(box.style.height).toBe("44px");
		await flip(DESKTOP_LAYOUT);
		expect(box.style.height).toBe("36px");
	});

	test("the text box refits when the web font arrives and a value now wraps", async () => {
		const fonts = loadingFonts();
		try {
			const { unmount } = await mount();
			const box = byRole("textbox", "Vendor") as HTMLTextAreaElement;
			Object.defineProperty(box, "scrollHeight", {
				configurable: true,
				get: () => 54,
			});
			await act(async () => {});
			expect(box.style.height).not.toBe("56px");
			await fonts.arrive();
			expect(box.style.height).toBe("56px");
			expect(fonts.listening()).toBe(1);
			await unmount();
			expect(fonts.listening()).toBe(0);
		} finally {
			fonts.restore();
		}
	});

	test("so does the JSON box", async () => {
		const { flip } = await mount(
			patchField("vendor_name", { kind: "json", required: false }),
		);
		const box = byRole("textbox", "Vendor") as HTMLTextAreaElement;
		measureLikeABrowser(box, 86, 94);
		await flip(PHONE_LAYOUT);
		expect(box.style.height).toBe("96px");
		await flip(DESKTOP_LAYOUT);
		expect(box.style.height).toBe("88px");
	});
});
