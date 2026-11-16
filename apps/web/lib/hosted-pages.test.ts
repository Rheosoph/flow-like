import { afterEach, describe, expect, it, mock, spyOn } from "bun:test";
import { onRequest } from "../functions/a/[[path]]";

afterEach(() => mock.restore());

function context(path = "/a/app-1/support", method = "GET") {
	return {
		request: new Request(`https://pages.example${path}`, {
			method,
			headers: { Authorization: "Bearer private", Cookie: "session=private" },
		}),
		env: {
			FLOW_LIKE_WEB_API_URL: "https://api.example/prefix/",
			ASSETS: {
				fetch: mock(
					async (_request: Request) =>
						new Response("<html>Static shell</html>"),
				),
			},
		},
	};
}

describe("hosted Pages document", () => {
	it("adds previews to query-form links without forwarding unrelated parameters", async () => {
		const fetcher = spyOn(globalThis, "fetch");
		for (const path of ["/a", "/a/"]) {
			fetcher.mockResolvedValueOnce(
				new Response("<title>App</title>", {
					headers: { "Content-Type": "text/html" },
				}),
			);
			const input = context(
				`${path}?app=app-1&route=%2Fhelp%20desk&token=private`,
			);
			expect(await (await onRequest(input)).text()).toBe("<title>App</title>");
			const [url, init] = fetcher.mock.calls[fetcher.mock.calls.length - 1];
			expect(String(url)).toBe(
				"https://api.example/prefix/frontend/document?app=app-1&route=%2Fhelp+desk",
			);
			expect(init?.headers).toEqual({ Accept: "text/html" });
		}
	});

	it("serves metadata in the first response without forwarding visitor credentials or queries", async () => {
		const fetcher = spyOn(globalThis, "fetch").mockResolvedValue(
			new Response(
				'<title>App</title><meta property="og:title" content="App">',
				{
					headers: {
						"Content-Type": "text/html",
						"Set-Cookie": "upstream=private",
						ETag: '"shell"',
					},
				},
			),
		);
		const input = context(
			"/a/app-1/help%20desk/deep?token=private&__variant=canary",
		);
		const response = await onRequest(input);
		expect(response.status).toBe(200);
		expect(await response.text()).toContain(
			'property="og:title" content="App"',
		);
		const [url, init] = fetcher.mock.calls[0];
		expect(String(url)).toBe(
			"https://api.example/prefix/frontend/document/app-1/help%20desk/deep",
		);
		expect(init?.headers).toEqual({ Accept: "text/html" });
		expect(init?.redirect).toBe("manual");
		expect(response.headers.get("Cache-Control")).toBe("no-store");
		expect(response.headers.has("Set-Cookie")).toBe(false);
		expect(response.headers.has("ETag")).toBe(false);
		expect(input.env.ASSETS.fetch).not.toHaveBeenCalled();
	});

	it("uses the static shell when the API is unavailable or the route is unpublished", async () => {
		const fetcher = spyOn(globalThis, "fetch");
		for (const status of [404, 500, 503, 307]) {
			fetcher.mockResolvedValueOnce(new Response("Unavailable", { status }));
			const input = context("/a/app-1?token=private");
			const response = await onRequest(input);
			expect(await response.text()).toBe("<html>Static shell</html>");
			const [assetRequest] = input.env.ASSETS.fetch.mock.calls[0];
			expect(assetRequest.url).toBe("https://pages.example/a");
			expect([...assetRequest.headers]).toEqual([]);
			expect(response.headers.get("Cache-Control")).toBe("no-store");
		}
		fetcher.mockRejectedValueOnce(new Error("Network unavailable"));
		expect(await (await onRequest(context())).text()).toContain("Static shell");
	});

	it("keeps the generic shell static and rejects mutations", async () => {
		const fetcher = spyOn(globalThis, "fetch");
		for (const path of ["/a", "/a/", "/a?app=&route=/help"])
			expect(await (await onRequest(context(path))).text()).toContain(
				"Static shell",
			);
		const input = context("/a/app-1", "POST");
		const response = await onRequest(input);
		expect(response.status).toBe(405);
		expect(response.headers.get("Allow")).toBe("GET, HEAD");
		expect(fetcher).not.toHaveBeenCalled();
		expect(input.env.ASSETS.fetch).not.toHaveBeenCalled();
	});

	it("returns metadata headers without a body for HEAD", async () => {
		spyOn(globalThis, "fetch").mockResolvedValue(
			new Response(null, { headers: { "Content-Type": "text/html" } }),
		);
		const response = await onRequest(context("/a/app-1", "HEAD"));
		expect(response.status).toBe(200);
		expect(response.headers.get("Content-Type")).toBe(
			"text/html; charset=utf-8",
		);
		expect(await response.text()).toBe("");
	});

	it("never fetches an invalid configured API URL", async () => {
		const fetcher = spyOn(globalThis, "fetch");
		for (const url of [
			"",
			"file:///local",
			"https://user:secret@api.example",
			"https://api.example?token=private",
		]) {
			const input = context();
			input.env.FLOW_LIKE_WEB_API_URL = url;
			expect(await (await onRequest(input)).text()).toContain("Static shell");
		}
		expect(fetcher).not.toHaveBeenCalled();
	});
});
