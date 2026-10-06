import { afterAll, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { createRuntimeResources } from "../lib/service-runtime/resources";

const window = new Window({ url: "https://studio.example.test" });
Object.assign(window, { SyntaxError, TypeError, Error });
const globals = {
	window,
	document: window.document,
	navigator: window.navigator,
	IS_REACT_ACT_ENVIRONMENT: true,
};
const original = Object.keys(globals).map(
	(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
);
Object.assign(globalThis, globals);
const { createRoot } = await import("react-dom/client");
const { AssetSourceResolverContext, useAssetSource } = await import(
	"./use-asset-source"
);
const host = window.document.createElement("div");
window.document.body.appendChild(host);
const root = createRoot(host as unknown as HTMLElement);
const requests: string[] = [];
const resources = createRuntimeResources({
	appId: "project",
	fetch: async (path) => {
		requests.push(path);
		return new Response("image", { headers: { "content-type": "image/png" } });
	},
});

function Probe({ source }: { source: string }) {
	const result = useAssetSource("device-runtime:test", source, {
		localFiles: true,
	});
	return (
		<output data-loading={result.isLoading}>{result.src ?? "blocked"}</output>
	);
}
async function render(source: string) {
	await act(async () => {
		root.render(
			<AssetSourceResolverContext.Provider value={resources.resolve}>
				<Probe source={source} />
			</AssetSourceResolverContext.Provider>,
		);
	});
}
afterAll(async () => {
	await act(() => root.unmount());
	resources.close();
	await window.happyDOM.abort();
	for (const [key, descriptor] of original) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
});

test("scoped assets never resolve a device path against the desktop viewer's filesystem or Studio origin", async () => {
	for (const path of [
		"/Users/viewer/private/model.glb",
		"C:\\Users\\viewer\\private.glb",
		"asset://localhost/private/data",
		"file:///private/data",
		"http://asset.localhost/private/data",
		"/images/local.png",
	]) {
		await render(path);
		expect(host.textContent).toBe("blocked");
		expect(host.querySelector("output")?.getAttribute("data-loading")).toBe(
			"false",
		);
	}
	await render("storage://models/model.glb");
	const loaded = host.textContent;
	expect(loaded?.startsWith("blob:")).toBe(true);
	expect(requests).toEqual(["/ui/assets?store=upload&path=models%2Fmodel.glb"]);
	await render("https://cdn.example.test/model.glb");
	expect(host.textContent).toBe("https://cdn.example.test/model.glb");
	await render("/Users/viewer/private.glb");
	expect(host.textContent).toBe("blocked");
	resources.close();
	await render("models/after-close.glb");
	expect(host.textContent).toBe("blocked");
});
