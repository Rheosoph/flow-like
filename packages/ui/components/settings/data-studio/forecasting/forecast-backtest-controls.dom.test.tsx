// @vitest-environment happy-dom
import React, { act, useState } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { forecastBacktestRange } from "./forecast-backtest";
import { ForecastBacktestControls } from "./forecast-backtest-controls";
import { formatForecastTime } from "./forecast-data";

const day = (index: number) => Date.UTC(2025, 0, index);
const history = Array.from({ length: 8 }, (_, index) => ({
	series_id: "north",
	target: "sales",
	timestamp_ms: day(index + 1),
	value: index + 10,
}));
const onChange = vi.fn();
const onCommit = vi.fn();
let root: Root;
let container: HTMLDivElement;

function Controls({ initialOffset = 0, running = false }) {
	const [offset, setOffset] = useState(initialOffset);
	return (
		<ForecastBacktestControls
			range={forecastBacktestRange(history, "sales", "north", 2, offset)}
			offset={offset}
			horizon={2}
			disabled={running}
			running={running}
			onChange={(value) => {
				onChange(value);
				setOffset(value);
			}}
			onCommit={onCommit}
		/>
	);
}

function required<T>(value: T | null | undefined): T {
	if (value === null || value === undefined)
		throw new Error("Expected test value");
	return value;
}
const thumb = () =>
	required(container.querySelector<HTMLElement>('[role="slider"]'));
const latest = () =>
	required(
		[...container.querySelectorAll<HTMLButtonElement>("button")].find(
			(button) => button.textContent === "Latest window",
		),
	);
async function render(initialOffset = 0, running = false) {
	await act(async () => {
		root.render(<Controls initialOffset={initialOffset} running={running} />);
	});
}
async function left() {
	await act(async () => {
		thumb().focus();
		thumb().dispatchEvent(
			new KeyboardEvent("keydown", {
				key: "ArrowLeft",
				bubbles: true,
				cancelable: true,
			}),
		);
	});
}

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true, React });
	onChange.mockClear();
	onCommit.mockClear();
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});
afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
});

describe("historical cutoff Radix controls", () => {
	it("commits an earlier offset with ArrowLeft and describes its dates accessibly", async () => {
		await render();
		expect(thumb().getAttribute("aria-valuenow")).toBe("4");
		expect(thumb().getAttribute("aria-valuetext")).toBe(
			`${formatForecastTime(day(6), true)} UTC, evaluating the next 2 steps`,
		);
		expect(
			document.getElementById(required(thumb().getAttribute("aria-labelledby")))
				?.textContent,
		).toBe("Holdout forecast as of");
		await left();
		expect(onChange).toHaveBeenCalledExactlyOnceWith(1);
		expect(onCommit).toHaveBeenCalledExactlyOnceWith(1);
		expect(thumb().getAttribute("aria-valuenow")).toBe("3");
		expect(thumb().getAttribute("aria-valuetext")).toBe(
			`${formatForecastTime(day(5), true)} UTC, evaluating the next 2 steps`,
		);
		expect(
			document.getElementById(
				required(thumb().getAttribute("aria-describedby")),
			)?.textContent,
		).toContain(
			`Compare ${formatForecastTime(day(6), true)} UTC to ${formatForecastTime(day(7), true)} UTC`,
		);
		expect(container.textContent).toContain(
			"Drag the cutoff and release to evaluate.",
		);
		expect(container.textContent).toContain(
			"Outlook continues from the latest observation.",
		);
		expect(container.textContent).toContain(
			"History: 5 stepsHoldout: 2 stepsLater: 1 step",
		);
	});

	it("supports Home and End to reach the earliest and latest windows", async () => {
		await render();
		for (const [key, offset] of [
			["Home", 4],
			["End", 0],
		] as const) {
			await act(async () =>
				thumb().dispatchEvent(
					new KeyboardEvent("keydown", {
						key,
						bubbles: true,
						cancelable: true,
					}),
				),
			);
			expect(onCommit).toHaveBeenLastCalledWith(offset);
		}
	});

	it("evaluates the selected window explicitly without moving the cutoff", async () => {
		await render(2);
		const evaluate = required(
			[...container.querySelectorAll<HTMLButtonElement>("button")].find(
				(button) => button.textContent === "Evaluate window",
			),
		);
		await act(async () => evaluate.click());
		expect(onCommit).toHaveBeenCalledExactlyOnceWith(2);
		expect(onChange).not.toHaveBeenCalled();
	});

	it("resets and commits the latest window in one click", async () => {
		await render(2);
		expect(latest().disabled).toBe(false);
		await act(async () => latest().click());
		expect(onChange).toHaveBeenCalledExactlyOnceWith(0);
		expect(onCommit).toHaveBeenCalledExactlyOnceWith(0);
		expect(thumb().getAttribute("aria-valuenow")).toBe("4");
		expect(latest().disabled).toBe(true);
	});

	it("blocks keyboard and reset actions while evaluation is running", async () => {
		await render(2, true);
		expect(thumb().closest('[aria-disabled="true"]')).not.toBeNull();
		expect(thumb().getAttribute("tabindex")).toBeNull();
		expect(latest().disabled).toBe(true);
		expect(container.textContent).toContain("Evaluating the selected window…");
		await left();
		await act(async () => latest().click());
		expect(onChange).not.toHaveBeenCalled();
		expect(onCommit).not.toHaveBeenCalled();
		expect(thumb().getAttribute("aria-valuenow")).toBe("2");
	});
});
