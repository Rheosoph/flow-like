import { afterAll, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { createRuntimeResources } from "./resources";

const window = new Window({ url: "https://studio.example.test" });
Object.assign(window, { SyntaxError, TypeError, Error });
// Happy DOM overrides nodeName on subclasses. DOMPurify reads the native Node getter.
const nodeName = Object.getOwnPropertyDescriptor(
	window.Node.prototype,
	"nodeName",
);
Object.defineProperty(window.Node.prototype, "nodeName", {
	configurable: true,
	get(this: Node) {
		if (this.nodeType === 1) return (this as Element).tagName;
		if (this.nodeType === 3) return "#text";
		if (this.nodeType === 8) return "#comment";
		if (this.nodeType === 9) return "#document";
		if (this.nodeType === 11) return "#document-fragment";
		return nodeName?.get?.call(this) ?? "";
	},
});
const previous = Object.getOwnPropertyDescriptor(globalThis, "window");
Object.assign(globalThis, { window });
afterAll(async () => {
	await window.happyDOM.abort();
	if (nodeName)
		Object.defineProperty(window.Node.prototype, "nodeName", nodeName);
	if (previous) Object.defineProperty(globalThis, "window", previous);
	else Reflect.deleteProperty(globalThis, "window");
});
const path = `/ui/assets/${"a".repeat(32)}`;
async function loaded(source: string, mime?: string) {
	const resources = createRuntimeResources({
		fetch: async () =>
			new Response(new TextEncoder().encode(source), {
				headers: mime ? { "content-type": mime } : {},
			}),
	});
	try {
		const url = await resources.resolve(path);
		const response = await fetch(url);
		return {
			type: response.headers.get("content-type"),
			text: await response.text(),
		};
	} finally {
		resources.close();
	}
}

describe("deployed asset content safety", () => {
	test("keeps passive SVG graphics while removing scripts, embedded HTML, animation and outbound references", async () => {
		const result = await loaded(
			`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" onload="alert(document.cookie)">
<defs><linearGradient id="paint"><stop offset="0" stop-color="red"/></linearGradient></defs>
<circle cx="10" cy="10" r="5" fill="url(#paint)" onmouseover="alert(1)"/>
<script>alert(document.cookie)</script><foreignObject><iframe srcdoc="&lt;script&gt;alert(1)&lt;/script&gt;"></iframe></foreignObject>
<style>@import 'https://attacker.test/private';</style><a href="javascript:alert(1)"><text>Click</text></a>
<image href="https://attacker.test/image"/><use href="https://attacker.test/remote.svg#x"/>
<animate attributeName="href" values="javascript:alert(1)"/>
<rect x="0" y="0" width="1" height="1" fill="url(https://attacker.test/image)" style="fill:url(https://attacker.test/image)"/>
</svg>`,
			"image/svg+xml;charset=UTF-8",
		);
		expect(result.type).toBe("image/svg+xml");
		expect(result.text).toContain("<svg");
		expect(result.text).toContain("<circle");
		expect(result.text).toContain("url(#paint)");
		expect(result.text).not.toMatch(
			/script|foreignObject|iframe|onload|onmouseover|<style|<a |<image|<use|<animate|attacker|javascript/i,
		);
	});

	test("rejects each active SVG construct independently", async () => {
		for (const markup of [
			"<script>alert(1)</script>",
			'<foreignObject><div xmlns="http://www.w3.org/1999/xhtml">unsafe</div></foreignObject>',
			'<image href="https://attacker.test/a.png"/>',
			'<use href="https://attacker.test/a.svg#x"/>',
			'<animate attributeName="href" values="javascript:alert(1)"/>',
			'<style>@import "https://attacker.test/a.css";</style>',
			'<circle r="1" fill="url(https://attacker.test/a.svg)" onload="alert(1)"/>',
		]) {
			const result = await loaded(
				`<svg xmlns="http://www.w3.org/2000/svg">${markup}</svg>`,
				"image/svg+xml",
			);
			expect(result.text).not.toMatch(
				/script|foreignObject|<image|<use|<animate|<style|attacker|onload/i,
			);
		}
	});

	test("HTML, JavaScript, XML and unknown assets remain downloadable with an inert MIME type", async () => {
		const source =
			'<html><script>window.opener.document.body.textContent="pwned"</script></html>';
		for (const mime of [
			"text/html",
			"application/xhtml+xml",
			"text/javascript",
			"application/javascript",
			"application/xml",
			"text/xml",
			"image/unknown",
			"custom/unknown",
			undefined,
		])
			expect(await loaded(source, mime)).toEqual({
				type: "application/octet-stream",
				text: source,
			});
	});

	test("recognized media, documents and text retain their bytes and explicit MIME type", async () => {
		for (const mime of [
			"image/png",
			"image/jpeg",
			"video/mp4",
			"audio/wav",
			"application/pdf",
			"text/plain",
			"application/json",
			"model/gltf-binary",
		]) {
			const result = await loaded("original bytes", `${mime}; charset=UTF-8`);
			expect(result.type?.split(";", 1)[0]).toBe(mime);
			expect(result.text).toBe("original bytes");
		}
	});
});
