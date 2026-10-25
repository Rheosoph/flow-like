// @vitest-environment happy-dom

import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import type { BoundValue, ForecastChartComponent } from "../types";

const state = vi.hoisted(() => ({
	appId: "context-project",
	renders: [] as Record<string, unknown>[],
}));

vi.mock("../ActionHandler", () => ({
	useActionContext: () => ({ appId: state.appId }),
}));
vi.mock("../DataContext", () => ({
	useData: () => ({
		resolve: (value: BoundValue) => {
			if ("literalString" in value) return value.literalString;
			if ("literalBool" in value) return value.literalBool;
			if ("literalNumber" in value) return value.literalNumber;
			return undefined;
		},
	}),
}));
vi.mock("../../settings/data-studio/forecasting/saved-forecast-view", () => ({
	SavedForecastView: (props: Record<string, unknown>) => {
		state.renders.push(props);
		return <div data-testid="saved-forecast">{String(props.forecastId)}</div>;
	},
}));

import { A2UIForecastChart } from "./ForecastChart";

let root: Root;
let host: HTMLDivElement;

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	state.appId = "context-project";
	state.renders = [];
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
});

afterEach(async () => {
	await act(() => root.unmount());
	host.remove();
});

const component: ForecastChartComponent = {
	id: "outlook",
	type: "forecastChart",
	forecastId: { literalString: "prediction-a" },
};

describe("A2UI saved forecast rendering", () => {
	test("delegates the scoped saved selection and forwards the editor element ref", async () => {
		const elementRef = vi.fn();
		const onAction = vi.fn();
		await act(() =>
			root.render(
				<A2UIForecastChart
					component={{
						...component,
						userScoped: { literalBool: true },
						view: { literalString: "evaluation" },
						showMetrics: { literalBool: false },
						showControls: { literalBool: false },
					}}
					componentId="outlook"
					surfaceId="page"
					appId="runtime-project"
					renderChild={() => null}
					elementRef={elementRef}
					onAction={onAction}
				/>,
			),
		);
		expect(state.renders.at(-1)).toMatchObject({
			appId: "runtime-project",
			forecastId: "prediction-a",
			userScoped: true,
			view: "evaluation",
			showMetrics: false,
			showControls: false,
		});
		expect(elementRef).toHaveBeenCalledWith(host.firstElementChild);
		expect(onAction).not.toHaveBeenCalled();
	});

	test("changes the reference when bindings change without placing values in page state", async () => {
		await act(() =>
			root.render(
				<A2UIForecastChart
					component={component}
					componentId="outlook"
					surfaceId="page"
					renderChild={() => null}
				/>,
			),
		);
		expect(state.renders.at(-1)?.appId).toBe("context-project");
		await act(() =>
			root.render(
				<A2UIForecastChart
					component={{
						...component,
						forecastId: { literalString: "prediction-b" },
						appId: { literalString: "owning-project" },
					}}
					componentId="outlook"
					surfaceId="page"
					renderChild={() => null}
				/>,
			),
		);
		expect(state.renders.at(-1)).toMatchObject({
			appId: "owning-project",
			forecastId: "prediction-b",
			userScoped: false,
		});
		expect(host.textContent).toBe("prediction-b");
	});

	test("does not mount a data reader until both project and snapshot are selected", async () => {
		state.appId = "";
		await act(() =>
			root.render(
				<A2UIForecastChart
					component={component}
					componentId="outlook"
					surfaceId="page"
					renderChild={() => null}
				/>,
			),
		);
		expect(host.textContent).toContain("inside a project");
		expect(state.renders).toEqual([]);
		await act(() =>
			root.render(
				<A2UIForecastChart
					component={{ ...component, forecastId: { literalString: "" } }}
					appId="project"
					componentId="outlook"
					surfaceId="page"
					renderChild={() => null}
				/>,
			),
		);
		expect(host.textContent).toContain("Select a saved forecast");
		expect(state.renders).toEqual([]);
	});
});
