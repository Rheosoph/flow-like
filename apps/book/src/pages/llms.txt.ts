import { getCollection } from "astro:content";
import type { APIRoute } from "astro";
import { renderLlmsTxt } from "../lib/llm-content";

export const GET: APIRoute = async () => {
	const entries = await getCollection("docs");

	return new Response(renderLlmsTxt(entries), {
		headers: {
			"Cache-Control": "public, max-age=3600, stale-while-revalidate=86400",
			"Content-Language": "en",
			"Content-Type": "text/markdown; charset=utf-8",
			Link: '</llms.txt>; rel="self"; type="text/markdown"',
			"X-Robots-Tag": "noindex, follow",
		},
	});
};
