import { describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { act, createElement } from "react";
import { useImagePreviewUrls } from "./image-previews";

async function setup() {
	const window = new Window();
	Object.assign(window, { SyntaxError, TypeError });
	Object.assign(globalThis, {
		document: window.document,
		Element: window.Element,
		Event: window.Event,
		HTMLElement: window.HTMLElement,
		Node: window.Node,
		navigator: window.navigator,
		window,
		Document: window.Document,
		DocumentFragment: window.DocumentFragment,
		Text: window.Text,
		File: window.File,
		IS_REACT_ACT_ENVIRONMENT: true,
	});
	const created: string[] = [];
	const revoked: string[] = [];
	URL.createObjectURL = () => {
		const url = `blob:test/${created.length}`;
		created.push(url);
		return url;
	};
	URL.revokeObjectURL = (url: string) => void revoked.push(url);
	const { createRoot } = await import("react-dom/client");
	const container = window.document.createElement("div");
	window.document.body.append(container);
	return { container, root: createRoot(container), created, revoked };
}

const image = (name: string) =>
	new File(["x"], name, { type: "image/png" }) as unknown as File;

/** Every value the hook returned for `file`, one entry per render. */
function probe(seen: (string | undefined)[]) {
	return function Probe({
		files,
		enabled,
		track,
	}: {
		files: File[];
		enabled?: boolean;
		track: File;
	}) {
		const urls = useImagePreviewUrls(files, enabled);
		seen.push(urls.get(track));
		return null;
	};
}

describe("useImagePreviewUrls", () => {
	test("a thumbnail has its src on the first render, not one frame later", async () => {
		const { root, created } = await setup();
		const seen: (string | undefined)[] = [];
		const Probe = probe(seen);
		const a = image("a.png");

		await act(async () => {
			root.render(createElement(Probe, { files: [a], track: a }));
		});

		expect(seen[0]).toBe(created[0]);
		expect(seen).not.toContain(undefined);
	});

	test("a surviving file keeps its URL when the list changes", async () => {
		const { root, created, revoked } = await setup();
		const seen: (string | undefined)[] = [];
		const Probe = probe(seen);
		const a = image("a.png");
		const b = image("b.png");

		await act(async () => {
			root.render(createElement(Probe, { files: [a], track: a }));
		});
		const first = seen[0];

		await act(async () => {
			root.render(createElement(Probe, { files: [a, b], track: a }));
		});

		expect(seen.at(-1)).toBe(first);
		expect(revoked).not.toContain(first);
		expect(created).toHaveLength(2);
	});

	test("a removed file's URL is revoked", async () => {
		const { root, created, revoked } = await setup();
		const seen: (string | undefined)[] = [];
		const Probe = probe(seen);
		const a = image("a.png");
		const b = image("b.png");

		await act(async () => {
			root.render(createElement(Probe, { files: [a, b], track: b }));
		});
		const aUrl = created[0];

		await act(async () => {
			root.render(createElement(Probe, { files: [b], track: b }));
		});

		expect(revoked).toEqual([aUrl]);
	});

	test("unmounting revokes everything", async () => {
		const { root, created, revoked } = await setup();
		const seen: (string | undefined)[] = [];
		const Probe = probe(seen);
		const a = image("a.png");

		await act(async () => {
			root.render(createElement(Probe, { files: [a], track: a }));
		});
		await act(async () => {
			root.unmount();
		});

		expect(revoked.sort()).toEqual([...created].sort());
	});

	test("disabled mints nothing and releases what it held", async () => {
		const { root, created, revoked } = await setup();
		const seen: (string | undefined)[] = [];
		const Probe = probe(seen);
		const a = image("a.png");

		await act(async () => {
			root.render(
				createElement(Probe, { files: [a], enabled: true, track: a }),
			);
		});
		expect(created).toHaveLength(1);

		await act(async () => {
			root.render(
				createElement(Probe, { files: [a], enabled: false, track: a }),
			);
		});

		expect(created).toHaveLength(1);
		expect(revoked).toEqual([created[0]]);
		expect(seen.at(-1)).toBeUndefined();
	});

	test("non-image files never mint a URL", async () => {
		const { root, created } = await setup();
		const seen: (string | undefined)[] = [];
		const Probe = probe(seen);
		const doc = new File(["x"], "notes.txt", {
			type: "text/plain",
		}) as unknown as File;

		await act(async () => {
			root.render(createElement(Probe, { files: [doc], track: doc }));
		});

		expect(created).toHaveLength(0);
		expect(seen.at(-1)).toBeUndefined();
	});
});
