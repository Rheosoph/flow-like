import { defineWidget } from "@flow-like/widget-sdk";

/** One revenue bucket — mirrors `SalesRow` in the package's Rust node */
export interface SalesRow {
	label: string;
	value: number;
	category: string;
}

interface Inputs {
	/** Headline above the chart @default "Revenue by month" */
	title: string;
	/** Rendering style @default "bar" */
	variant: "bar" | "line";
	/** Revenue rows to plot @default [] */
	rows: SalesRow[];
	/** Currency symbol shown next to values @default "€" */
	currency: string;
	/** Label of the bucket to emphasise; empty highlights nothing @default "" */
	highlight: string;
}

interface Events {
	/** A bucket was clicked */
	pointSelected: { label: string; value: number; index: number };
	/** The user asked the flow for fresh data */
	// biome-ignore lint/suspicious/noConfusingVoidType: contract convention for payload-less events
	refreshRequested: void;
}

interface Queries {
	/** Currently selected bucket; `selected` is false while nothing is picked */
	getSelection: {
		// biome-ignore lint/suspicious/noConfusingVoidType: contract convention for argument-less queries
		args: void;
		returns: { label: string; value: number; index: number; selected: boolean };
	};
	/** The N highest-revenue rows the widget currently displays */
	getSeries: { args: { top: number }; returns: SalesRow[] };
	/** Sum of every displayed row */
	// biome-ignore lint/suspicious/noConfusingVoidType: contract convention for argument-less queries
	getTotal: { args: void; returns: number };
}

export default defineWidget<Inputs, Events, Queries>({
	id: "sales-chart",
	name: "Sales Chart",
	description:
		"Interactive revenue chart. Instantiate it with rows, update them live from a flow, and read the current selection or series back through queries.",
	sizing: { defaultHeight: 340, resizable: true, maxHeight: 900 },
	dev: {
		fixtures: {
			empty: { title: "No data yet", rows: [] },
			loaded: {
				title: "Q3 revenue",
				rows: [
					{ label: "Jul", value: 4200, category: "Hardware" },
					{ label: "Aug", value: 5100, category: "Software" },
					{ label: "Sep", value: 3800, category: "Services" },
				],
			},
		},
	},
});
