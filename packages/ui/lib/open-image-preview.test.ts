import { expect, test } from "bun:test";
import { Window } from "happy-dom";
import { openImagePreview } from "./open-image-preview";

test("image popouts preserve the image URL as data and remove their opener", () => {
	const popup = new Window();
	Object.defineProperty(popup, "opener", {
		configurable: true,
		writable: true,
		value: {},
	});
	const previous = Object.getOwnPropertyDescriptor(globalThis, "window");
	Object.defineProperty(globalThis, "window", {
		configurable: true,
		value: { open: () => popup },
	});
	try {
		const url =
			'data:image/png;base64,AAAA" /><script>attack()</script><img onerror="attack()';
		openImagePreview(url);
		expect(popup.opener).toBeNull();
		expect(popup.document.body.children.length).toBe(1);
		const image = popup.document.body.firstElementChild;
		if (!(image instanceof popup.HTMLElement))
			throw new Error("Expected an image in the popup");
		expect(image.tagName).toBe("IMG");
		expect(image.getAttribute("src")).toBe(url);
		expect(image.getAttribute("onerror")).toBeNull();
		expect(image.style.maxWidth).toBe("100%");
	} finally {
		if (previous) Object.defineProperty(globalThis, "window", previous);
		else Reflect.deleteProperty(globalThis, "window");
		popup.happyDOM.abort();
	}
});

test("a blocked popup does not throw", () => {
	const previous = Object.getOwnPropertyDescriptor(globalThis, "window");
	Object.defineProperty(globalThis, "window", {
		configurable: true,
		value: { open: () => null },
	});
	try {
		expect(() => openImagePreview("data:image/png;base64,AAAA")).not.toThrow();
	} finally {
		if (previous) Object.defineProperty(globalThis, "window", previous);
		else Reflect.deleteProperty(globalThis, "window");
	}
});
