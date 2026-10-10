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
import { type ComponentProps, act } from "react";
import { type ILayer, ILayerType } from "../../../lib/schema/flow/board";
import { BoardBreadcrumb } from "./board-breadcrumb";

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

function layer(
	id: string,
	name: string,
	parent_id: string | null = null,
	type = ILayerType.Collapsed,
): ILayer {
	return { id, name, parent_id, type } as ILayer;
}

const layers: Record<string, ILayer> = {
	outer: layer("outer", "Prepare"),
	inner: layer("inner", "Transform", "outer"),
	checkout: layer("checkout", "Checkout", null, ILayerType.Module),
	payments: layer("payments", "Payments", null, ILayerType.Module),
	charge: layer("charge", "Charge", "payments", ILayerType.Function),
	refund: layer("refund", "Refund", "payments", ILayerType.Function),
	order: layer("order", "Order", "checkout", ILayerType.Function),
	shared: layer("shared", "Shared", null, ILayerType.Function),
};

function render(props: Partial<ComponentProps<typeof BoardBreadcrumb>> = {}) {
	const onJumpToLayer = mock((_path: string) => {});
	const onReturnToVisit = mock((_index: number) => {});
	act(() =>
		root.render(
			<BoardBreadcrumb
				layers={layers}
				onJumpToLayer={onJumpToLayer}
				onReturnToVisit={onReturnToVisit}
				{...props}
			/>,
		),
	);
	return {
		onJumpToLayer,
		onReturnToVisit,
		buttons: Array.from(container.querySelectorAll("button")),
	};
}

describe("file breadcrumbs", () => {
	test("keeps main-file ancestor destinations and disables the current layer", () => {
		const { buttons, onJumpToLayer } = render({
			layerPath: "outer/inner",
		});
		expect(buttons.map((button) => button.textContent)).toEqual([
			"main.flow",
			"Prepare",
			"Transform",
		]);
		act(() => {
			for (const button of buttons) button.click();
		});
		expect(onJumpToLayer.mock.calls).toEqual([["root"], ["outer"]]);
		expect(buttons[2].disabled).toBe(true);
		expect(buttons[2].getAttribute("aria-current")).toBe("page");
	});

	test("returns from a module function to that module", () => {
		const { buttons, onJumpToLayer } = render({
			layerPath: "payments/charge",
		});
		expect(buttons.map((button) => button.textContent)).toEqual([
			"payments.flow",
			"Charge",
		]);
		act(() => buttons[0].click());
		expect(onJumpToLayer.mock.calls).toEqual([["payments"]]);
	});

	test("shows only nested-module contents and keeps full ancestor paths", () => {
		const { buttons, onJumpToLayer } = render({
			layers: {
				...layers,
				payments: { ...layers.payments, parent_id: "checkout" },
				outer: { ...layers.outer, parent_id: "payments" },
			},
			layerPath: "checkout/payments/outer/inner",
		});
		expect(buttons.map((button) => button.textContent)).toEqual([
			"checkout/payments.flow",
			"Prepare",
			"Transform",
		]);
		act(() => {
			buttons[0].click();
			buttons[1].click();
		});
		expect(onJumpToLayer.mock.calls).toEqual([
			["checkout/payments"],
			["checkout/payments/outer"],
		]);
	});

	test.each([
		{},
		{ layerPath: "root" },
		{ layerPath: "payments" },
		{
			layers: {
				...layers,
				payments: { ...layers.payments, parent_id: "checkout" },
			},
			layerPath: "checkout/payments",
		},
	])("renders nothing at file root %j", (props) => {
		render(props);
		expect(container.querySelector("nav")).toBeNull();
	});

	test("ignores history that does not end at the current location", () => {
		const { buttons, onJumpToLayer, onReturnToVisit } = render({
			layerPath: "payments/charge",
			navigationTrail: [{ from: "checkout/order", to: "shared" }],
		});
		expect(buttons.map((button) => button.textContent)).toEqual([
			"payments.flow",
			"Charge",
		]);
		act(() => buttons[0].click());
		expect(onJumpToLayer.mock.calls).toEqual([["payments"]]);
		expect(onReturnToVisit).not.toHaveBeenCalled();
	});

	test("shows the caller across modules and returns to its visit", () => {
		const { buttons, onJumpToLayer, onReturnToVisit } = render({
			layerPath: "payments/charge",
			navigationTrail: [{ from: "checkout/order", to: "payments/charge" }],
		});
		expect(buttons.map((button) => button.textContent)).toEqual([
			"checkout.flow",
			"Order",
			"payments.flow: Charge",
		]);
		act(() => {
			buttons[0].click();
			buttons[1].click();
		});
		expect(onJumpToLayer.mock.calls).toEqual([["checkout"]]);
		expect(onReturnToVisit.mock.calls).toEqual([[0]]);
		expect(buttons[2].disabled).toBe(true);
	});

	test("uses visit positions for repeated functions", () => {
		const { buttons, onReturnToVisit } = render({
			layerPath: "checkout/order",
			navigationTrail: [
				{ from: "checkout", to: "checkout/order" },
				{ from: "checkout/order", to: "payments/charge" },
				{ from: "payments/charge", to: "checkout/order" },
			],
		});
		expect(buttons.map((button) => button.textContent)).toEqual([
			"checkout.flow",
			"Order",
			"payments.flow: Charge",
			"checkout.flow: Order",
		]);
		act(() => {
			for (const button of buttons) button.click();
		});
		expect(onReturnToVisit.mock.calls).toEqual([[0], [1], [2]]);
	});

	test("keeps main as the originating root and qualifies calls back to main", () => {
		const { buttons, onReturnToVisit } = render({
			layerPath: "shared",
			navigationTrail: [
				{ from: undefined, to: "payments/charge" },
				{ from: "payments/charge", to: "shared" },
			],
		});
		expect(buttons.map((button) => button.textContent)).toEqual([
			"main.flow",
			"payments.flow: Charge",
			"main.flow: Shared",
		]);
		act(() => buttons[0].click());
		expect(onReturnToVisit.mock.calls).toEqual([[0]]);
	});

	test("keeps ancestors before the caller and omits repeated module qualifiers", () => {
		const { buttons, onJumpToLayer, onReturnToVisit } = render({
			layerPath: "payments/refund",
			navigationTrail: [
				{ from: "outer/inner", to: "payments/charge" },
				{ from: "payments/charge", to: "payments/refund" },
			],
		});
		expect(buttons.map((button) => button.textContent)).toEqual([
			"main.flow",
			"Prepare",
			"Transform",
			"payments.flow: Charge",
			"Refund",
		]);
		act(() => {
			buttons[1].click();
			buttons[2].click();
			buttons[3].click();
		});
		expect(onJumpToLayer.mock.calls).toEqual([["outer"]]);
		expect(onReturnToVisit.mock.calls).toEqual([[0], [1]]);
	});

	test("keeps the route visible when a visit ends at a module root", () => {
		const { buttons } = render({
			layerPath: "payments",
			navigationTrail: [{ from: "checkout/order", to: "payments" }],
		});
		expect(buttons.map((button) => button.textContent)).toEqual([
			"checkout.flow",
			"Order",
			"payments.flow",
		]);
	});
});
