import { describe, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { GEOMETRY_BOARD_FORMAT_VERSION } from "../../../lib/board-format";
import { geometryMarker } from "../../../lib/geometry";
import {
	IValueType,
	type IVariable,
	IVariableType,
} from "../../../lib/schema/flow/variable";
import { convertJsonToUint8Array } from "../../../lib/uint8";
import type { IBackendState } from "../../../state/backend-state";

mock.module("../../ui/geometry-editor-map", () => ({ default: () => null }));

async function setup() {
	const window = new Window();
	Object.assign(window, { SyntaxError, TypeError });
	Object.assign(globalThis, {
		document: window.document,
		Element: window.Element,
		Event: window.Event,
		HTMLElement: window.HTMLElement,
		HTMLTextAreaElement: window.HTMLTextAreaElement,
		HTMLInputElement: window.HTMLInputElement,
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
	const { GeometryValueInput } = await import("./geometry-variable");
	const container = window.document.createElement("div");
	window.document.body.append(container);
	return {
		window,
		container,
		root: createRoot(container as unknown as HTMLElement),
		GeometryValueInput,
	};
}

type Queryable = { querySelectorAll(selectors: string): ArrayLike<unknown> };

const buttonByText = (container: Queryable, text: string) =>
	Array.from(
		container.querySelectorAll("button") as ArrayLike<HTMLButtonElement>,
	).find((button) => button.textContent?.trim() === text);

const numberInputs = (container: Queryable) =>
	Array.from(
		container.querySelectorAll(
			'input[type="number"]',
		) as ArrayLike<HTMLInputElement>,
	);

describe("Geometry editor rendering", () => {
	test("hydrates geometry defaults without sending an update command", async () => {
		const { container, root } = await setup();
		const { VariablesMenuEdit } = await import("./variables-menu-edit");
		const updates: IVariable[] = [];
		const variable: IVariable = {
			id: "location",
			name: "Location",
			data_type: IVariableType.Geometry,
			value_type: IValueType.Normal,
			default_value: null,
			exposed: false,
			secret: true,
			editable: true,
		};
		const render = async (next: IVariable) => {
			await act(async () =>
				root.render(
					<VariablesMenuEdit
						variable={next}
						updateVariable={async (updated) => {
							updates.push(updated);
						}}
					/>,
				),
			);
		};
		const input = () => container.querySelector('input[type="password"]');
		await render(variable);
		expect(input()?.value).toBe("");
		const point = { type: "Point", coordinates: [13, 52] };
		await render({
			...variable,
			default_value: convertJsonToUint8Array(point),
		});
		expect(input()?.value).toBe(JSON.stringify(point));
		await render(variable);
		expect(input()?.value).toBe("");
		expect(updates).toEqual([]);
		await act(async () => root.unmount());
	});

	test("clearing a secret geometry sends an explicit null default", async () => {
		const { container, root } = await setup();
		const { GeometryVariable } = await import("./geometry-variable");
		const updates: IVariable[] = [];
		const variable: IVariable = {
			id: "location",
			name: "Location",
			data_type: IVariableType.Geometry,
			value_type: IValueType.Normal,
			default_value: convertJsonToUint8Array({
				type: "Point",
				coordinates: [13, 52],
			}),
			exposed: false,
			secret: true,
			editable: true,
		};
		await act(async () =>
			root.render(
				<GeometryVariable
					variable={variable}
					onChange={(updated) => updates.push(updated)}
				/>,
			),
		);
		await act(async () =>
			buttonByText(container, "Show geometry value")?.click(),
		);
		await act(async () => buttonByText(container, "Clear")?.click());
		expect(updates).toHaveLength(1);
		expect(updates[0].default_value).toEqual(convertJsonToUint8Array(null));
		await act(async () => root.unmount());
	});

	test("falls back to JSON with the subtype error and preserves the invalid draft", async () => {
		const { container, root, GeometryValueInput } = await setup();
		await act(async () => {
			root.render(
				<GeometryValueInput
					value={{
						type: "LineString",
						coordinates: [
							[0, 0],
							[1, 1],
						],
					}}
					schema={geometryMarker("Point")}
					preview={false}
					onChange={() => {}}
				/>,
			);
		});
		expect(
			container.querySelector("textarea")?.getAttribute("aria-invalid"),
		).toBe("true");
		expect(container.querySelector('[role="alert"]')?.textContent).toContain(
			"Point",
		);
		expect(container.querySelector("textarea")?.value).toContain("LineString");
		await act(async () => root.unmount());
	});

	test("renders unset values without inventing a coordinate and conceals secrets", async () => {
		const { container, root, GeometryValueInput } = await setup();
		await act(async () =>
			root.render(
				<GeometryValueInput
					value={null}
					schema={geometryMarker("Point")}
					preview={false}
					onChange={() => {}}
				/>,
			),
		);
		expect(container.querySelector("textarea")).toBeNull();
		expect(container.querySelector('[role="alert"]')).toBeNull();
		expect(numberInputs(container).map((input) => input.value)).toEqual([
			"",
			"",
		]);
		await act(async () =>
			root.render(
				<GeometryValueInput
					value={{ type: "Point", coordinates: [13, 52] }}
					secret
					preview={false}
					onChange={() => {}}
				/>,
			),
		);
		expect(numberInputs(container)).toHaveLength(0);
		expect(container.querySelector('input[type="password"]')).not.toBeNull();
		await act(async () => container.querySelector("button")?.click());
		expect(numberInputs(container).map((input) => input.value)).toEqual([
			"13",
			"52",
		]);
		await act(async () => root.unmount());
	});

	test("vertex rows add and remove positions and emit normalized geometry", async () => {
		const { container, root, GeometryValueInput } = await setup();
		const emitted: unknown[] = [];
		await act(async () =>
			root.render(
				<GeometryValueInput
					value={{
						type: "LineString",
						coordinates: [
							[0, 0],
							[1, 1],
						],
					}}
					preview={false}
					onChange={(value, valid) => emitted.push(valid ? value : "invalid")}
				/>,
			),
		);
		expect(numberInputs(container)).toHaveLength(4);
		await act(async () => buttonByText(container, "Add vertex")?.click());
		expect(emitted.at(-1)).toEqual({
			type: "LineString",
			coordinates: [
				[0, 0],
				[1, 1],
				[1, 1],
			],
		});
		expect(numberInputs(container)).toHaveLength(6);
		const remove = container.querySelectorAll(
			'button[aria-label="Remove vertex"]',
		);
		expect(remove).toHaveLength(3);
		await act(async () => (remove[0] as HTMLButtonElement).click());
		await act(async () => {
			container
				.querySelectorAll('button[aria-label="Remove vertex"]')[0]
				?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
		});
		expect(emitted.at(-1)).toBe("invalid");
		expect(container.querySelector('[role="alert"]')?.textContent).toContain(
			"at least 2 positions",
		);
		await act(async () => root.unmount());
	});

	test("polygons expose rings and empty holes are not emitted", async () => {
		const { container, root, GeometryValueInput } = await setup();
		const emitted: unknown[] = [];
		const square = [
			[0, 0],
			[1, 0],
			[1, 1],
			[0, 1],
			[0, 0],
		];
		await act(async () =>
			root.render(
				<GeometryValueInput
					value={{ type: "Polygon", coordinates: [square] }}
					preview={false}
					onChange={(value, valid) => emitted.push(valid ? value : "invalid")}
				/>,
			),
		);
		expect(container.textContent).toContain("Exterior ring");
		expect(numberInputs(container)).toHaveLength(8);
		await act(async () => buttonByText(container, "Add hole")?.click());
		expect(container.textContent).toContain("Hole 1");
		expect(emitted.at(-1)).toEqual({ type: "Polygon", coordinates: [square] });
		await act(async () =>
			(
				container.querySelector(
					'button[aria-label="Remove hole"]',
				) as HTMLButtonElement | null
			)?.click(),
		);
		expect(container.textContent).not.toContain("Hole 1");
		await act(async () => root.unmount());
	});

	test("JSON mode shows the current value as text", async () => {
		const { container, root, GeometryValueInput } = await setup();
		await act(async () =>
			root.render(
				<GeometryValueInput
					value={{ type: "Point", coordinates: [13, 52] }}
					preview={false}
					onChange={() => {}}
				/>,
			),
		);
		expect(container.querySelector("textarea")).toBeNull();
		await act(async () => buttonByText(container, "JSON")?.click());
		expect(container.querySelector("textarea")?.value).toContain('"Point"');
		await act(async () => buttonByText(container, "Map")?.click());
		expect(container.querySelector("textarea")).toBeNull();
		expect(numberInputs(container)).toHaveLength(2);
		await act(async () => root.unmount());
	});
});

test("Geometry creation requires a compatible backend board format", async () => {
	const { container, root } = await setup();
	const { useBoardFormat } = await import("../../../hooks/use-board-format");
	const { useBackendStore } = await import("../../../state/backend-state");
	const previous = useBackendStore.getState().backend;
	const probe = (appId: string) => {
		function Probe() {
			return (
				<span>
					{useBoardFormat(appId) >= GEOMETRY_BOARD_FORMAT_VERSION
						? "enabled"
						: "disabled"}
				</span>
			);
		}
		return <Probe />;
	};
	try {
		await act(async () => {
			useBackendStore.setState({
				backend: { boardState: {} } as unknown as IBackendState,
			});
			root.render(probe("old"));
		});
		expect(container.textContent).toBe("disabled");
		await act(async () => {
			useBackendStore.setState({
				backend: {
					boardState: {
						getBoardFormat: async (appId: string) => ({
							board_format_version: appId === "new" ? 2 : 1,
						}),
					},
				} as unknown as IBackendState,
			});
			root.render(probe("new"));
		});
		expect(container.textContent).toBe("enabled");
		await act(async () => root.render(probe("old")));
		expect(container.textContent).toBe("disabled");
	} finally {
		await act(async () => root.unmount());
		useBackendStore.setState({ backend: previous });
	}
});

test("board format negotiation ignores stale responses after switching apps", async () => {
	const { container, root } = await setup();
	const { useBoardFormat } = await import("../../../hooks/use-board-format");
	const { useBackendStore } = await import("../../../state/backend-state");
	const previous = useBackendStore.getState().backend;
	let resolveFirst!: (value: { board_format_version: number }) => void;
	const first = new Promise<{ board_format_version: number }>((resolve) => {
		resolveFirst = resolve;
	});
	function Probe({ appId }: { appId: string }) {
		return <span>{useBoardFormat(appId)}</span>;
	}
	try {
		await act(async () => {
			useBackendStore.setState({
				backend: {
					boardState: {
						getBoardFormat: async (appId: string) => {
							if (appId === "first") return first;
							if (appId === "offline") throw new Error("Unavailable");
							return { board_format_version: 3 };
						},
					},
				} as unknown as IBackendState,
			});
			root.render(<Probe appId="first" />);
		});
		expect(container.textContent).toBe("1");
		await act(async () => root.render(<Probe appId="second" />));
		expect(container.textContent).toBe("3");
		await act(async () => resolveFirst({ board_format_version: 2 }));
		expect(container.textContent).toBe("3");
		await act(async () => root.render(<Probe appId="offline" />));
		expect(container.textContent).toBe("1");
	} finally {
		await act(async () => root.unmount());
		useBackendStore.setState({ backend: previous });
	}
});
