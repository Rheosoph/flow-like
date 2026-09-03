import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { type ReactNode, act, createRef } from "react";
import { createRoot } from "react-dom/client";
import { MessageShell } from "./message-shell";

type Entry = { isIntersecting: boolean; target: Element };

class FakeIntersectionObserver {
	static instances: FakeIntersectionObserver[] = [];
	readonly targets = new Set<Element>();
	disconnected = false;

	constructor(
		readonly callback: (entries: Entry[]) => void,
		readonly options?: IntersectionObserverInit,
	) {
		FakeIntersectionObserver.instances.push(this);
	}

	observe(target: Element) {
		this.targets.add(target);
	}

	unobserve(target: Element) {
		this.targets.delete(target);
	}

	disconnect() {
		this.disconnected = true;
		this.targets.clear();
	}

	intersect(isIntersecting: boolean) {
		this.callback(
			[...this.targets].map((target) => ({ isIntersecting, target })),
		);
	}
}

const globals = globalThis as { IntersectionObserver?: unknown };

function installDom() {
	const window = new Window();
	Object.assign(window, { SyntaxError, TypeError });
	Object.assign(globalThis, {
		document: window.document,
		HTMLElement: window.HTMLElement,
		Node: window.Node,
		navigator: window.navigator,
		requestAnimationFrame: window.requestAnimationFrame.bind(window),
		cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
		window,
		IS_REACT_ACT_ENVIRONMENT: true,
	});
	const element = window.document.createElement("div");
	window.document.body.append(element);
	// happy-dom's element types are structurally compatible with, but not
	// identical to, the lib.dom ones react-dom is typed against.
	const container = element as unknown as HTMLElement;
	return { container, root: createRoot(container) };
}

async function mount(ui: ReactNode) {
	const dom = installDom();
	await act(async () => {
		dom.root.render(ui);
	});
	return dom;
}

const placeholderOf = (container: HTMLElement) =>
	container.querySelector("[aria-busy]");

beforeEach(() => {
	FakeIntersectionObserver.instances = [];
	globals.IntersectionObserver = FakeIntersectionObserver;
});

// The fake must not leak into other files: LazyPlateStatic relies on the
// no-observer fallback under test.
afterEach(() => {
	globals.IntersectionObserver = undefined;
});

describe("MessageShell", () => {
	test("renders a placeholder until the row intersects, then stays mounted", async () => {
		const { container } = await mount(
			<MessageShell>
				<p>hello</p>
			</MessageShell>,
		);

		expect(placeholderOf(container)).not.toBeNull();
		expect(container.textContent).not.toContain("hello");
		expect(FakeIntersectionObserver.instances).toHaveLength(1);
		const [observer] = FakeIntersectionObserver.instances;
		expect(observer.targets.size).toBe(1);

		await act(async () => observer.intersect(false));
		expect(container.textContent).not.toContain("hello");

		await act(async () => observer.intersect(true));
		expect(container.textContent).toContain("hello");
		expect(placeholderOf(container)).toBeNull();
		expect(observer.disconnected).toBe(true);

		await act(async () =>
			observer.callback([
				{ isIntersecting: false, target: container as unknown as Element },
			]),
		);
		expect(container.textContent).toContain("hello");
		expect(FakeIntersectionObserver.instances).toHaveLength(1);
	});

	test("immediate mounts on the first render without observing", async () => {
		const { container } = await mount(
			<MessageShell immediate>
				<p>hello</p>
			</MessageShell>,
		);

		expect(container.textContent).toContain("hello");
		expect(placeholderOf(container)).toBeNull();
		expect(FakeIntersectionObserver.instances).toHaveLength(0);
	});

	test("observes against the scroll container when one is given", async () => {
		const scroller = createRef<HTMLDivElement>();
		const { container } = await mount(
			<div ref={scroller}>
				<MessageShell root={scroller}>
					<p>hello</p>
				</MessageShell>
			</div>,
		);

		const [observer] = FakeIntersectionObserver.instances;
		expect(scroller.current).not.toBeNull();
		expect(observer.options?.root).toBe(scroller.current);
		expect(observer.options?.rootMargin).toBe("150% 0px");
		expect(container.textContent).not.toContain("hello");
	});

	test("mounts immediately when IntersectionObserver is unavailable", async () => {
		globals.IntersectionObserver = undefined;

		const { container } = await mount(
			<MessageShell>
				<p>hello</p>
			</MessageShell>,
		);

		expect(container.textContent).toContain("hello");
		expect(placeholderOf(container)).toBeNull();
		expect(FakeIntersectionObserver.instances).toHaveLength(0);
	});
});
