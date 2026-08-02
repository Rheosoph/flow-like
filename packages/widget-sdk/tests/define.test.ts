import { describe, expect, test } from "bun:test";
import {
	type QueryArgs,
	type QueryReturns,
	type WidgetEventsOf,
	type WidgetInputsOf,
	type WidgetQueriesOf,
	defineWidget,
} from "../src/define";

interface SalesRow {
	x: string;
	y: number;
}

interface Inputs {
	title: string;
	variant: "bar" | "line";
	limit: number;
	rows: SalesRow[];
}

interface Events {
	pointSelected: SalesRow;
	// biome-ignore lint/suspicious/noConfusingVoidType: `void` payloads are the documented authoring style for payload-less events
	refreshRequested: void;
}

interface Queries {
	// biome-ignore lint/suspicious/noConfusingVoidType: `void` args are the documented authoring style for argument-less queries
	getSelection: { args: void; returns: { rows: SalesRow[] } };
	// biome-ignore lint/suspicious/noConfusingVoidType: `void` args are the documented authoring style for argument-less queries
	getValue: { args: void; returns: string };
}

type Equal<A, B> = (<T>() => T extends A ? 1 : 2) extends <T>() => T extends B
	? 1
	: 2
	? true
	: false;

describe("defineWidget", () => {
	const definition = defineWidget<Inputs, Events, Queries>({
		id: "sales-chart",
		name: "Sales Chart",
		description: "Interactive bar/line chart",
		sizing: { defaultHeight: 320, resizable: true },
		dev: {
			fixtures: {
				empty: { rows: [] },
				loaded: { title: "Q3 Sales", limit: 10 },
			},
		},
	});

	test("returns the config unchanged", () => {
		expect(definition.id).toBe("sales-chart");
		expect(definition.name).toBe("Sales Chart");
		expect(definition.description).toBe("Interactive bar/line chart");
		expect(definition.sizing).toEqual({ defaultHeight: 320, resizable: true });
		expect(definition.dev?.fixtures?.empty).toEqual({ rows: [] });
	});

	test("carries no runtime phantom field", () => {
		expect("__types" in definition).toBe(false);
	});

	test("type-level: generics survive on the definition", () => {
		const inputsMatch: Equal<WidgetInputsOf<typeof definition>, Inputs> = true;
		const eventsMatch: Equal<WidgetEventsOf<typeof definition>, Events> = true;
		const queriesMatch: Equal<
			WidgetQueriesOf<typeof definition>,
			Queries
		> = true;
		expect(inputsMatch).toBe(true);
		expect(eventsMatch).toBe(true);
		expect(queriesMatch).toBe(true);
	});

	test("type-level: query args and returns resolve", () => {
		const args: Equal<QueryArgs<Queries, "getSelection">, void> = true;
		const returns: Equal<
			QueryReturns<Queries, "getSelection">,
			{ rows: SalesRow[] }
		> = true;
		const value: Equal<QueryReturns<Queries, "getValue">, string> = true;
		expect(args).toBe(true);
		expect(returns).toBe(true);
		expect(value).toBe(true);
	});

	test("works without type arguments", () => {
		const loose = defineWidget({
			id: "kpi-card",
			name: "KPI Card",
			description: "Single KPI",
		});
		expect(loose.sizing).toBeUndefined();
		expect(loose.dev).toBeUndefined();
	});
});
