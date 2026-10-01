import { defineMiddleware } from "astro:middleware";

const securityHeaders: Record<string, string> = {
	"Strict-Transport-Security": "max-age=31536000; includeSubDomains; preload",
	"X-Frame-Options": "DENY",
	"X-Content-Type-Options": "nosniff",
	"Referrer-Policy": "strict-origin-when-cross-origin",
	"Permissions-Policy":
		"camera=(), microphone=(), geolocation=(), payment=(), usb=(), magnetometer=(), gyroscope=(), accelerometer=()",
	"Content-Security-Policy": [
		"default-src 'self'",
		"script-src 'self' 'unsafe-inline' https://challenges.cloudflare.com",
		"style-src 'self' 'unsafe-inline'",
		"connect-src 'self' https://api.github.com https://api.flow-like.com https://650afa0c.sibforms.com",
		"img-src 'self' data: https:",
		"font-src 'self' data:",
		"media-src 'self'",
		"frame-src https://challenges.cloudflare.com",
		"object-src 'none'",
		"frame-ancestors 'none'",
		"base-uri 'self'",
		"form-action 'self' https://650afa0c.sibforms.com",
		"upgrade-insecure-requests",
	].join("; "),
};

export const onRequest = defineMiddleware(async (context, next) => {
	let response = await next();
	if (import.meta.env.DEV && response.status === 404) {
		// Worker-first routing also catches Vite modules during development.
		// Let the asset binding serve them after Astro finds no page.
		const { env } = await import("cloudflare:workers");
		const asset = await env.ASSETS.fetch(context.request);
		if (asset.status !== 404) response = asset;
	}
	const headers = new Headers(response.headers);

	if (!import.meta.env.DEV) {
		for (const [key, value] of Object.entries(securityHeaders)) {
			headers.set(key, value);
		}
		if (
			/^\/(?:thirdparty\/|desktop\/)?callback(?:\/|\.html)?$/.test(
				context.url.pathname,
			)
		) {
			headers.set("Referrer-Policy", "no-referrer");
		}
	}

	headers.delete("X-Powered-By");

	return new Response(response.body, {
		status: response.status,
		statusText: response.statusText,
		headers,
	});
});
