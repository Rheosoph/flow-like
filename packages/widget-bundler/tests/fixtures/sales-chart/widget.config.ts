import { defineWidget } from "@flow-like/widget-sdk";
import type { SalesRow } from "./types";

interface Inputs {
	/** Chart headline @default "Sales" */
	title: string;
	/** Chart style @default "bar" */
	variant: "bar" | "line";
	/** Max points @minimum 1 @maximum 500 @default 50 */
	limit: number;
	rows: SalesRow[];
	/** Show the legend */
	showLegend?: boolean;
}

interface Events {
	/** Fired when a data point is clicked */
	pointSelected: SalesRow;
	// biome-ignore lint/suspicious/noConfusingVoidType: contract convention for payload-less events
	refreshRequested: void;
}

interface Queries {
	// biome-ignore lint/suspicious/noConfusingVoidType: contract convention for argument-less queries
	getSelection: { args: void; returns: { rows: SalesRow[] } };
	// biome-ignore lint/suspicious/noConfusingVoidType: contract convention for argument-less queries
	getValue: { args: void; returns: string };
}

export default defineWidget<Inputs, Events, Queries>({
	id: "sales-chart",
	name: "Sales Chart",
	description: "Interactive bar/line chart",
	sizing: { defaultHeight: 320, resizable: true },
	dev: {
		fixtures: {
			empty: { rows: [] },
			loaded: { title: "Q3 Sales" },
		},
	},
});
