import {
	afterAll,
	afterEach,
	beforeEach,
	describe,
	expect,
	mock,
	test,
} from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import type { Lesson, LessonAssetView } from "../../lib/learn/types";

mock.module("next/dynamic", () => ({
	default: () => () => null,
}));

mock.module("../ui/text-editor", () => ({
	TextEditor: ({ initialContent }: { initialContent: string }) => (
		<div data-slate-editor data-initial-content={initialContent}>
			<figure data-testid="generated-caption">
				<img
					src="https://assets.example.test/app-anatomy.webp?sig=fresh"
					alt=""
				/>
			</figure>
			<figure data-testid="authored-caption">
				<img
					src="https://assets.example.test/authored.webp"
					alt="Authored diagram"
				/>
				<figcaption>Purpose-written caption</figcaption>
			</figure>
		</div>
	),
}));

const lesson: Lesson = {
	id: "lesson-1",
	module_id: "module-1",
	title: "App anatomy",
	position: 0,
	language: "en",
	content: "# App anatomy\n\nThe lesson starts here.",
	video_url: null,
	estimated_minutes: 8,
	is_optional: false,
};

const imageAsset: LessonAssetView = {
	id: "asset-1",
	name: "AppAnatomy",
	mime_type: "image/webp",
	kind: "IMAGE",
	signed_url: "https://assets.example.test/app-anatomy.webp?sig=fresh",
};

let browserWindow: Window;
let host: HTMLElement;
let root: Root;

beforeEach(() => {
	browserWindow = new Window({ url: "https://learn.flow-like.test" });
	// happy-dom leaves this intrinsic unset under Bun; selectors use it when
	// reporting parser errors, so install the host intrinsic before React mounts.
	Object.assign(browserWindow, { SyntaxError });
	Object.assign(globalThis, {
		document: browserWindow.document,
		Element: browserWindow.Element,
		Event: browserWindow.Event,
		HTMLElement: browserWindow.HTMLElement,
		HTMLImageElement: browserWindow.HTMLImageElement,
		Node: browserWindow.Node,
		navigator: browserWindow.navigator,
		requestAnimationFrame:
			browserWindow.requestAnimationFrame.bind(browserWindow),
		cancelAnimationFrame:
			browserWindow.cancelAnimationFrame.bind(browserWindow),
		window: browserWindow,
		IS_REACT_ACT_ENVIRONMENT: true,
	});
	host = browserWindow.document.createElement("div") as unknown as HTMLElement;
	browserWindow.document.body.append(host);
	root = createRoot(host);
});

afterEach(async () => {
	await act(async () => root.unmount());
	browserWindow.close();
});

afterAll(() => mock.restore());

async function renderLesson() {
	const { LessonContent } = await import("./lesson-content");
	await act(async () => {
		root.render(<LessonContent lesson={lesson} assets={[imageAsset]} />);
	});
}

describe("lesson content DOM", () => {
	test("renders one lesson title and passes a de-duplicated body to the editor", async () => {
		await renderLesson();

		const titles = host.querySelectorAll("h1");
		expect(titles).toHaveLength(1);
		expect(titles[0]?.textContent).toBe("App anatomy");
		expect(
			host
				.querySelector("[data-slate-editor]")
				?.getAttribute("data-initial-content"),
		).toBe("The lesson starts here.");
	});

	test("exposes the stable prose hook inside a wide article canvas", async () => {
		await renderLesson();

		const article = host.querySelector("article");
		expect(article?.classList.contains("fl-lesson-article")).toBe(true);
		expect(article?.classList.contains("max-w-5xl")).toBe(true);
		expect(article?.classList.contains("max-w-3xl")).toBe(false);
		expect(host.querySelector(".fl-lesson-prose")?.getAttribute("class")).toBe(
			"fl-lesson-prose",
		);
	});

	test("adds a readable caption, preserves authored captions, and exposes failures", async () => {
		await renderLesson();

		const generated = host.querySelector(
			'figure[data-testid="generated-caption"]',
		) as HTMLElement;
		const generatedImage = generated.querySelector("img") as HTMLImageElement;
		await act(async () => {
			generatedImage.dispatchEvent(
				new browserWindow.Event("load", { bubbles: true }),
			);
		});
		expect(generatedImage.alt).toBe("App Anatomy");
		expect(generated.dataset.lessonMediaCaption).toBe("App Anatomy");

		const authored = host.querySelector(
			'figure[data-testid="authored-caption"]',
		) as HTMLElement;
		const authoredImage = authored.querySelector("img") as HTMLImageElement;
		await act(async () => {
			authoredImage.dispatchEvent(
				new browserWindow.Event("load", { bubbles: true }),
			);
		});
		expect(authored.dataset.lessonMediaCaption).toBeUndefined();
		expect(authored.querySelector("figcaption")?.textContent).toBe(
			"Purpose-written caption",
		);

		await act(async () => {
			generatedImage.dispatchEvent(
				new browserWindow.Event("error", { bubbles: true }),
			);
		});
		expect(generatedImage.dataset.lessonMediaFailed).toBe("true");
		expect(generated.dataset.lessonMediaFailed).toBe("true");
		expect(generated.dataset.lessonMediaCaption).toBeUndefined();
		expect(generated.getAttribute("role")).toBe("img");
		expect(generated.getAttribute("aria-label")).toBe(
			"App Anatomy could not be loaded.",
		);
	});
});
