import { getCollection } from "astro:content";
import type { APIRoute, GetStaticPaths } from "astro";
import {
	type LlmBookEntry,
	bookMarkdownPath,
	renderBookEntryMarkdown,
} from "../../lib/llm-content";
import {
	BOOK_ORIGIN,
	bookEntryPath,
	normalizeBookEntryId,
} from "../../lib/seo";

interface MarkdownPageProps {
	readonly entry: LlmBookEntry;
}

export const getStaticPaths: GetStaticPaths = async () => {
	const entries = await getCollection("docs");
	return entries.map((entry) => ({
		params: { slug: normalizeBookEntryId(entry.id) || undefined },
		props: { entry } satisfies MarkdownPageProps,
	}));
};

export const GET: APIRoute = async ({ props }) => {
	const { entry } = props as MarkdownPageProps;
	const entries = await getCollection("docs");
	const canonical = new URL(bookEntryPath(entry.id), BOOK_ORIGIN).toString();
	const markdown = new URL(bookMarkdownPath(entry.id), BOOK_ORIGIN).toString();

	return new Response(renderBookEntryMarkdown(entry, entries), {
		headers: {
			"Cache-Control": "public, max-age=3600, stale-while-revalidate=86400",
			"Content-Language": "en",
			"Content-Type": "text/markdown; charset=utf-8",
			Link: `<${canonical}>; rel="canonical", <${BOOK_ORIGIN}/llms.txt>; rel="describedby", <${markdown}>; rel="self"; type="text/markdown"`,
		},
	});
};
