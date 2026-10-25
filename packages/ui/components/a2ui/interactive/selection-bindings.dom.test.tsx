// @vitest-environment happy-dom

import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DataProvider } from "../DataContext";
import { applyElementUpdate } from "../apply-a2ui-message";
import type {
	DataEntry,
	RadioGroupComponent,
	SelectComponent,
	SurfaceComponent,
} from "../types";
import { A2UIRadioGroup } from "./RadioGroup";
import { A2UISelect } from "./Select";

vi.mock("../ActionHandler", () => ({
	useComponentEventTrigger: () => async () => {},
	useOnAction: () => undefined,
}));
vi.mock("../../../lib", async () => import("../../../lib/utils"));

const options = [
	{ value: "a", label: "Option A" },
	{ value: "b", label: "Option B" },
];
let host: HTMLDivElement;
let root: Root;

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	host = document.createElement("div");
	document.body.appendChild(host);
	root = createRoot(host);
});

afterEach(async () => {
	await act(async () => root.unmount());
	host.remove();
});

async function render(element: SurfaceComponent, data: DataEntry[] = []) {
	const props = {
		componentId: element.id,
		surfaceId: "page",
		renderChild: () => null,
	};
	await act(async () => {
		root.render(
			<DataProvider initialData={data}>
				<form>
					{element.component.type === "select" ? (
						<A2UISelect
							{...props}
							component={element.component as SelectComponent}
						/>
					) : (
						<A2UIRadioGroup
							{...props}
							component={element.component as RadioGroupComponent}
						/>
					)}
				</form>
			</DataProvider>,
		);
	});
}

describe.each(["select", "radioGroup"] as const)("%s bindings", (type) => {
	function element(): SurfaceComponent {
		return {
			id: "selection",
			component: {
				id: "selection",
				type,
				value: { literalString: "a" },
				options: { literalOptions: options },
				disabled: { literalBool: false },
			},
		};
	}

	function expectDisabled(disabled: boolean) {
		const controls = host.querySelectorAll<HTMLButtonElement>(
			type === "select" ? '[role="combobox"]' : '[role="radio"]',
		);
		expect(controls.length).toBe(type === "select" ? 1 : options.length);
		for (const control of controls) expect(control.disabled).toBe(disabled);
	}

	function expectOptions(labels: string[]) {
		const rendered = host.querySelectorAll(
			type === "select" ? "select option" : "label",
		);
		expect(Array.from(rendered, (option) => option.textContent)).toEqual(
			labels,
		);
	}

	it("applies disable and enable updates while retaining literal options", async () => {
		let current = element();
		await render(current);
		expectDisabled(false);
		expectOptions(["Option A", "Option B"]);

		current = applyElementUpdate(current, {
			type: "setDisabled",
			disabled: true,
		});
		await render(current);
		expectDisabled(true);
		expectOptions(["Option A", "Option B"]);

		current = applyElementUpdate(current, {
			type: "setDisabled",
			disabled: false,
		});
		await render(current);
		expectDisabled(false);
		expectOptions(["Option A", "Option B"]);
	});

	it("resolves options and disabled state from data paths", async () => {
		const current = element();
		current.component = {
			...current.component,
			value: { path: "/value" },
			options: { path: "/options" },
			disabled: { path: "/disabled" },
		} as SelectComponent | RadioGroupComponent;
		await render(current, [
			{ path: "/value", value: "a" },
			{ path: "/options", value: options },
			{ path: "/disabled", value: true },
		]);
		expectDisabled(true);
		expectOptions(["Option A", "Option B"]);

		await render(current, [
			{ path: "/value", value: "updated-a" },
			{
				path: "/options",
				value: options.map((option) => ({
					value: `updated-${option.value}`,
					label: `Updated ${option.label}`,
				})),
			},
			{ path: "/disabled", value: false },
		]);
		expectDisabled(false);
		expectOptions(["Updated Option A", "Updated Option B"]);
	});
});
