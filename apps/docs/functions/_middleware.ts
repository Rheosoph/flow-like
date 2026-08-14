/**
 * Content negotiation for agents: `Accept: text/markdown` (or a `.md` suffix)
 * serves the Markdown twin that the build wrote next to every page, while
 * browsers keep getting HTML.
 */

import {
	serveMarkdown,
	withVaryAccept,
} from "../scripts/agent-markdown/markdown-negotiation.mjs";

interface PagesContext {
	request: Request;
	next: (input?: Request | string, init?: RequestInit) => Promise<Response>;
}

export async function onRequest({
	request,
	next,
}: PagesContext): Promise<Response> {
	// Always a GET, so a HEAD request still measures the real document.
	const markdown = await serveMarkdown(request, (path: string) =>
		next(new URL(path, request.url).toString()),
	);
	if (markdown) return markdown;

	return withVaryAccept(await next());
}
