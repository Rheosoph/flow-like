import {
	afterAll,
	afterEach,
	beforeEach,
	describe,
	expect,
	test,
} from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { ApiResponseError } from "../../../lib/api-error";
import { BoardLoadError } from "./board-load-error";

const testWindow = new Window({ url: "https://localhost" });
Object.assign(testWindow, { SyntaxError, TypeError, Error });
const domGlobals = {
	window: testWindow,
	document: testWindow.document,
	navigator: testWindow.navigator,
	HTMLElement: testWindow.HTMLElement,
	Element: testWindow.Element,
	Node: testWindow.Node,
	IS_REACT_ACT_ENVIRONMENT: true,
};
const previousGlobals = new Map(
	Object.keys(domGlobals).map((key) => [
		key,
		Object.getOwnPropertyDescriptor(globalThis, key),
	]),
);
const installDom = () => Object.assign(globalThis, domGlobals);
installDom();

const { createRoot } = await import("react-dom/client");
let root: ReturnType<typeof createRoot>;
let container: HTMLDivElement;

beforeEach(() => {
	installDom();
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(() => {
	act(() => root.unmount());
	container.remove();
});

afterAll(() => {
	for (const [key, descriptor] of previousGlobals) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
	testWindow.happyDOM.abort();
});

const back = { href: "/library/config/flows?id=app", label: "Back to flows" };
const retryButton = () =>
	[...container.querySelectorAll("button")].find(
		(button) => button.textContent === "Retry",
	);

describe("BoardLoadError", () => {
	test("a deleted board links back to the app's flows and offers no retry", () => {
		act(() =>
			root.render(
				<BoardLoadError
					error={
						new ApiResponseError({
							status: 404,
							code: "NOT_FOUND",
							message: "Board board not found",
						})
					}
					retrying={false}
					onRetry={() => {}}
					back={back}
				/>,
			),
		);
		expect(container.textContent).toContain("This flow no longer exists");
		expect(container.querySelector("a")?.getAttribute("href")).toBe(back.href);
		expect(retryButton()).toBeUndefined();
	});

	test("a failed read shows the cause and retries until it settles", () => {
		let retries = 0;
		const render = (retrying: boolean) =>
			act(() =>
				root.render(
					<BoardLoadError
						error={new Error("[HTTP_502] Bad Gateway")}
						retrying={retrying}
						onRetry={() => retries++}
						back={back}
					/>,
				),
			);
		render(false);
		expect(container.textContent).toContain("The flow could not be loaded.");
		expect(container.textContent).toContain("[HTTP_502] Bad Gateway");
		act(() => retryButton()?.click());
		expect(retries).toBe(1);
		render(true);
		expect(retryButton()?.disabled).toBe(true);
		expect(container.querySelector("a")?.textContent).toBe("Back to flows");
	});

	test("an embedded board leaves navigation to its host", () => {
		act(() =>
			root.render(
				<BoardLoadError
					error={new Error("offline")}
					retrying={false}
					onRetry={() => {}}
				/>,
			),
		);
		expect(container.querySelector("a")).toBeNull();
		expect(retryButton()).toBeDefined();
	});
});
