import { afterEach, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { type Ref, act, createRef } from "react";
import { type Root, createRoot } from "react-dom/client";
import { useElementRef } from "./use-element-ref";

let root: Root | undefined;
let restoreGlobals: (() => void) | undefined;

function mountHost() {
	const window = new Window();
	const globals = {
		window,
		document: window.document,
		IS_REACT_ACT_ENVIRONMENT: true,
	};
	const previous = Object.fromEntries(
		Object.keys(globals).map((key) => [
			key,
			Object.getOwnPropertyDescriptor(globalThis, key),
		]),
	);
	Object.assign(globalThis, globals);
	restoreGlobals = () => {
		for (const [key, descriptor] of Object.entries(previous)) {
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		}
	};
	const host = window.document.createElement("div");
	window.document.body.append(host);
	root = createRoot(host as unknown as HTMLElement);
	return host;
}

afterEach(async () => {
	await act(() => root?.unmount());
	root = undefined;
	restoreGlobals?.();
	restoreGlobals = undefined;
});

function Element({
	localRef,
	elementRef,
}: {
	localRef: Ref<HTMLDivElement>;
	elementRef: (element: HTMLElement | SVGElement | null) => void;
}) {
	const ref = useElementRef(elementRef, localRef);
	return <div ref={ref} />;
}

describe("useElementRef", () => {
	test("shares the rendered root with an object ref without detaching on rerender", async () => {
		const host = mountHost();
		const localRef = createRef<HTMLDivElement>();
		const calls: (HTMLElement | SVGElement | null)[] = [];
		const elementRef = (element: HTMLElement | SVGElement | null) => {
			calls.push(element);
		};
		await act(() => {
			root?.render(<Element localRef={localRef} elementRef={elementRef} />);
		});
		const node = host.firstChild as unknown as HTMLDivElement;
		expect(localRef.current).toBe(node);
		expect(calls).toEqual([node]);
		await act(() => {
			root?.render(<Element localRef={localRef} elementRef={elementRef} />);
		});
		expect(calls).toEqual([node]);
		await act(() => root?.unmount());
		root = undefined;
		expect(localRef.current).toBeNull();
		expect(calls.at(-1)).toBeNull();
	});

	test("runs callback ref cleanup when the editor ref changes and on unmount", async () => {
		const host = mountHost();
		const localCalls: (HTMLDivElement | null | "cleanup")[] = [];
		const editorCalls: (HTMLElement | SVGElement | null)[] = [];
		const localRef = (element: HTMLDivElement | null) => {
			localCalls.push(element);
			return () => {
				localCalls.push("cleanup");
			};
		};
		const elementRef = (element: HTMLElement | SVGElement | null) => {
			editorCalls.push(element);
		};
		await act(() => {
			root?.render(<Element localRef={localRef} elementRef={elementRef} />);
		});
		const node = host.firstChild as unknown as HTMLDivElement;
		await act(() => {
			root?.render(<Element localRef={localRef} elementRef={() => {}} />);
		});
		expect(editorCalls).toEqual([node, null]);
		expect(localCalls).toEqual([node, "cleanup", node]);
		await act(() => root?.unmount());
		root = undefined;
		expect(localCalls).toEqual([node, "cleanup", node, "cleanup"]);
	});
});
