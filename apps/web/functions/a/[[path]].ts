interface HostedPagesContext {
	request: Request;
	env: {
		FLOW_LIKE_WEB_API_URL: string;
		ASSETS: { fetch(request: Request): Promise<Response> };
	};
}

/** Only the initial hosted document uses the API; all bundles remain static. */
export async function onRequest({
	request,
	env,
}: HostedPagesContext): Promise<Response> {
	if (request.method !== "GET" && request.method !== "HEAD") {
		return new Response("Method not allowed", {
			status: 405,
			headers: { Allow: "GET, HEAD", "Cache-Control": "no-store" },
		});
	}
	const url = new URL(request.url);
	const queryRouting = url.pathname === "/a" || url.pathname === "/a/";
	const app = queryRouting ? url.searchParams.get("app") : null;
	const suffix = url.pathname.slice("/a/".length);
	let response: Response | undefined;
	if (app || (!queryRouting && suffix)) {
		try {
			const api = new URL(env.FLOW_LIKE_WEB_API_URL);
			if (
				!/^https?:$/.test(api.protocol) ||
				api.username ||
				api.password ||
				api.search ||
				api.hash
			)
				throw new Error("Invalid hosted API origin");
			const base = `${api.pathname.replace(/\/+$/, "")}/frontend/document`;
			api.pathname = queryRouting ? base : `${base}/${suffix}`;
			if (app) {
				api.searchParams.set("app", app);
				api.searchParams.set("route", url.searchParams.get("route") || "/");
			}
			// Viewer credentials and navigation queries are never part of a preview.
			const upstream = await fetch(api, {
				method: request.method,
				headers: { Accept: "text/html" },
				redirect: "manual",
				signal: AbortSignal.timeout(8_000),
			});
			if (
				upstream.ok &&
				upstream.headers.get("Content-Type")?.includes("text/html")
			) {
				response = upstream;
			} else {
				await upstream.body?.cancel();
			}
		} catch {
			// The static client can still show its normal connection or route error.
		}
	}
	if (!response) {
		const shell = new URL("/a", url);
		response = await env.ASSETS.fetch(
			new Request(shell, { method: request.method }),
		);
	}
	// Do not inherit API cookies, CORS headers, or validators for a different body.
	return new Response(request.method === "HEAD" ? null : response.body, {
		status: response.status,
		headers: {
			"Content-Type": "text/html; charset=utf-8",
			"Cache-Control": "no-store",
			"X-Content-Type-Options": "nosniff",
			"Referrer-Policy": "strict-origin-when-cross-origin",
			"X-XSS-Protection": "0",
			"Content-Security-Policy-Report-Only":
				"script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval' blob:; object-src 'none'; base-uri 'self'",
		},
	});
}
