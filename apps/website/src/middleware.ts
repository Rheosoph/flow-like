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

export const onRequest = defineMiddleware(async (_context, next) => {
	const response = await next();
	const headers = new Headers(response.headers);

	if (!import.meta.env.DEV) {
		for (const [key, value] of Object.entries(securityHeaders)) {
			headers.set(key, value);
		}
	}

	headers.delete("X-Powered-By");

	return new Response(response.body, {
		status: response.status,
		statusText: response.statusText,
		headers,
	});
});
