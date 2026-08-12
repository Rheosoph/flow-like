import { describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import type { ProcessedAttachment } from "./attachment";

async function setup() {
	const window = new Window();
	Object.assign(window, { SyntaxError, TypeError });
	Object.assign(globalThis, {
		document: window.document,
		Element: window.Element,
		Event: window.Event,
		HTMLElement: window.HTMLElement,
		MouseEvent: window.MouseEvent,
		Node: window.Node,
		navigator: window.navigator,
		window,
		Document: window.Document,
		DocumentFragment: window.DocumentFragment,
		Text: window.Text,
		getComputedStyle: window.getComputedStyle.bind(window),
		requestAnimationFrame: window.requestAnimationFrame.bind(window),
		cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
		IS_REACT_ACT_ENVIRONMENT: true,
	});
	const { createRoot } = await import("react-dom/client");
	const container = window.document.createElement("div");
	window.document.body.append(container);
	return { container, root: createRoot(container) };
}

function attachment(
	overrides: Partial<ProcessedAttachment> & { url: string },
): ProcessedAttachment {
	const displayName = overrides.displayName ?? overrides.name ?? overrides.url;
	return {
		name: displayName,
		displayName,
		ext: displayName.split(".").slice(1).pop() ?? "",
		type: "document",
		isDataUrl: false,
		...overrides,
	};
}

/** Strips of more than six files start folded — open them before asserting. */
async function unfold(container: {
	textContent: string;
	querySelector: (s: string) => unknown;
}) {
	if (!container.textContent.includes("attachments")) return;
	await act(async () => {
		(container.querySelector("button") as HTMLElement | null)?.click();
	});
}

const image = (index: number) =>
	attachment({
		url: `https://cdn.test/photo-${index}.png`,
		displayName: `photo-${index}.png`,
		type: "image",
		size: 1_000_000,
	});

describe("AttachmentStrip", () => {
	test("a pdf and a markdown file render as the same compact chip", async () => {
		const { AttachmentStrip } = await import("./attachment-strip");
		const { container, root } = await setup();

		await act(async () => {
			root.render(
				<AttachmentStrip
					files={[
						attachment({
							url: "https://cdn.test/contract.pdf",
							displayName: "contract.pdf",
							type: "pdf",
							size: 4_800_000,
							previewText: "§7 Lieferfristen",
						}),
						attachment({
							url: "https://cdn.test/notes.md",
							displayName: "notes.md",
							type: "document",
							size: 8_400,
						}),
					]}
					onFileClick={() => {}}
				/>,
			);
		});

		const chips = container.querySelectorAll("button.group.h-9");
		expect(chips.length).toBe(2);
		// The preview text no longer claims a block of its own — it is the tooltip.
		expect(container.textContent).not.toContain("§7 Lieferfristen");
		expect(container.textContent).toContain("contract.pdf");
		expect(container.textContent).toContain("notes.md");
	});

	test("overflowing visuals collapse into a +N tile that reaches the dialog", async () => {
		const { AttachmentStrip } = await import("./attachment-strip");
		const { container, root } = await setup();
		let opened = 0;

		await act(async () => {
			root.render(
				<AttachmentStrip
					files={[1, 2, 3, 4, 5, 6, 7].map(image)}
					onFileClick={() => {}}
					onShowAll={() => {
						opened += 1;
					}}
				/>,
			);
		});

		await unfold(container);

		expect(container.textContent).toContain("+3");
		const tiles = container.querySelectorAll("button.group.relative");
		expect(tiles.length).toBe(4);

		await act(async () => {
			(tiles[3] as unknown as HTMLElement).click();
		});
		expect(opened).toBe(1);
	});

	test("overflowing chips keep a +N more control", async () => {
		const { AttachmentStrip } = await import("./attachment-strip");
		const { container, root } = await setup();

		await act(async () => {
			root.render(
				<AttachmentStrip
					files={Array.from({ length: 8 }, (_, i) =>
						attachment({
							url: `https://cdn.test/file-${i}.csv`,
							displayName: `file-${i}.csv`,
							size: 1_000,
						}),
					)}
					onFileClick={() => {}}
					onShowAll={() => {}}
				/>,
			);
		});

		await unfold(container);

		expect(container.textContent).toContain("+3 more");
	});

	test("the header states the real count and only offers Show all when files are hidden", async () => {
		const { AttachmentStrip } = await import("./attachment-strip");
		const { container, root } = await setup();

		await act(async () => {
			root.render(
				<AttachmentStrip
					files={[image(1), image(2), image(3)]}
					onFileClick={() => {}}
					onShowAll={() => {}}
				/>,
			);
		});
		expect(container.textContent).toContain("3 files");
		expect(container.textContent).not.toContain("Show all");

		await act(async () => {
			root.render(
				<AttachmentStrip
					files={[image(1), image(2), image(3), image(4), image(5)]}
					onFileClick={() => {}}
					onShowAll={() => {}}
				/>,
			);
		});
		expect(container.textContent).toContain("5 files");
		expect(container.textContent).toContain("Show all");
	});

	test("more than six attachments start folded and expand in place", async () => {
		const { AttachmentStrip } = await import("./attachment-strip");
		const { container, root } = await setup();

		const files = [
			...[1, 2, 3, 4, 5].map(image),
			attachment({
				url: "https://cdn.test/notes.md",
				displayName: "notes.md",
				size: 8_400,
			}),
			attachment({
				url: "https://cdn.test/data.csv",
				displayName: "data.csv",
				size: 1_200,
			}),
		];

		await act(async () => {
			root.render(
				<AttachmentStrip
					files={files}
					onFileClick={() => {}}
					onShowAll={() => {}}
				/>,
			);
		});

		expect(container.textContent).toContain("7 attachments");
		expect(container.textContent).toContain("5 images");
		expect(container.querySelectorAll("button.group.relative").length).toBe(0);

		await act(async () => {
			container.querySelector("button")?.click();
		});

		expect(container.querySelectorAll("button.group.relative").length).toBe(4);
		expect(container.textContent).toContain("Show less");
	});

	test("an encoded path renders as the decoded basename with its extension pinned", async () => {
		const { AttachmentStrip } = await import("./attachment-strip");
		const { getDisplayFileName } = await import("./attachment");
		const { container, root } = await setup();

		const raw = "%2FUsers%2Ffelix%2FDownloads%2FQ3%20report%20(v2).pdf";
		await act(async () => {
			root.render(
				<AttachmentStrip
					files={[
						attachment({
							url: "https://cdn.test/x",
							name: raw,
							displayName: getDisplayFileName(raw),
							ext: "pdf",
							type: "pdf",
						}),
					]}
					onFileClick={() => {}}
				/>,
			);
		});

		expect(container.textContent).toContain("Q3 report (v2)");
		expect(container.textContent).not.toContain("%2F");
	});
});
