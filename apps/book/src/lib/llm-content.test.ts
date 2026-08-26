import { describe, expect, it } from "bun:test";
import {
	type LlmBookEntry,
	bookMarkdownPath,
	bookMarkdownUrl,
	renderBookEntryMarkdown,
	renderLlmsFullTxt,
	renderLlmsTxt,
} from "./llm-content";

const introduction: LlmBookEntry = {
	id: "introduction",
	data: {
		title: "Introduction: One Program, Two Ways to See It",
		description: "Learn the FlowScript mental model.",
		seo: { topics: ["FlowScript", "visual workflows"] },
	},
	body: `import { Aside } from '@astrojs/starlight/components';
import WorkflowFigure from '../../components/WorkflowFigure.astro';

Read [the first chapter](/part-1/01-the-3-am-call/).

<Aside type="note" title="Precise contract">
The runtime executes the persisted Board.
</Aside>

<WorkflowFigure
  src={workflow}
  alt="A typed workflow graph."
  caption="Data and execution remain visibly separate."
/>

<IncidentDeskDemo client:load />`,
};

const chapter: LlmBookEntry = {
	id: "part-1/01-the-3-am-call",
	data: {
		title: "1. The 3 A.M. Call",
		description: "See why critical software needs to explain itself.",
		seo: { topics: ["explainable software"] },
	},
	body: "## The incident\n\nThe failing operation should have been visible.",
};

const home: LlmBookEntry = {
	id: "index",
	data: {
		title: "FlowBook: A Developer's Guide to Flow-Like",
		description: "Learn Flow-Like FlowScript.",
		seo: { topics: ["FlowScript"] },
	},
	body: "import BookHome from '../../components/BookHome.astro';\n\n<BookHome />",
};

const entries = [home, introduction, chapter] as const;

describe("FlowBook Markdown routes", () => {
	it("uses the v2 index.md convention beside trailing-slash HTML pages", () => {
		expect(bookMarkdownPath("index")).toBe("/index.md");
		expect(bookMarkdownPath("contents")).toBe("/contents/index.md");
		expect(bookMarkdownUrl(chapter.id)).toBe(
			"https://book.flow-like.com/part-1/01-the-3-am-call/index.md",
		);
	});
});

describe("FlowBook Markdown rendering", () => {
	it("turns supported MDX into standalone semantic Markdown", () => {
		const markdown = renderBookEntryMarkdown(introduction, entries);

		expect(markdown.match(/^# /gm)).toHaveLength(1);
		expect(markdown).not.toContain("import ");
		expect(markdown).not.toContain("<Aside");
		expect(markdown).not.toContain("WorkflowFigure");
		expect(markdown).not.toContain("client:load");
		expect(markdown).toContain("> **Precise contract**");
		expect(markdown).toContain(
			"> **Workflow figure:** A typed workflow graph.",
		);
		expect(markdown).toContain(
			"[the first chapter](https://book.flow-like.com/part-1/01-the-3-am-call/index.md)",
		);
		expect(markdown).toContain(
			"[Open the interactive version](https://book.flow-like.com/introduction/)",
		);
	});

	it("renders a useful home page instead of exposing an empty component shell", () => {
		const markdown = renderBookEntryMarkdown(home, entries);

		expect(markdown).toContain(
			"FlowScript is Flow-Like's typed textual language",
		);
		expect(markdown).toContain(
			"[Canonical web edition](https://book.flow-like.com/)",
		);
		expect(markdown).not.toContain("<BookHome");
	});

	it("fails closed when a new MDX component has no Markdown representation", () => {
		expect(() =>
			renderBookEntryMarkdown(
				{ ...chapter, body: "<UnknownBookWidget />" },
				entries,
			),
		).toThrow("Unsupported MDX component");
	});

	it("leaves MDX-like examples inside fenced code blocks untouched", () => {
		const body = [
			"## Literal MDX example",
			"",
			"```mdx",
			"<UnknownBookWidget />",
			"[Local route](/part-1/01-the-3-am-call/)",
			"```",
		].join("\n");
		const markdown = renderBookEntryMarkdown({ ...chapter, body }, entries);

		expect(markdown).toContain("<UnknownBookWidget />");
		expect(markdown).toContain("[Local route](/part-1/01-the-3-am-call/)");
	});
});

describe("LLM indexes", () => {
	it("produces the llms.txt v2 heading, summary, sections, and absolute links", () => {
		const llms = renderLlmsTxt(entries);

		expect(llms.match(/^# /gm)).toHaveLength(1);
		expect(llms).toStartWith("# FlowBook\n\n> ");
		expect(llms).toContain("## Start here");
		expect(llms).toContain("## Optional");
		expect(llms).toContain(bookMarkdownUrl(introduction.id));
		expect(llms).toContain("https://book.flow-like.com/llms-full.txt");
	});

	it("builds the full companion from published reading units only", () => {
		const full = renderLlmsFullTxt(entries);

		expect(full).toContain("# FlowBook: Complete Markdown Edition");
		expect(full).toContain(introduction.data.title);
		expect(full).toContain(chapter.data.title);
		expect(full).not.toContain(home.data.description ?? "");
	});
});
