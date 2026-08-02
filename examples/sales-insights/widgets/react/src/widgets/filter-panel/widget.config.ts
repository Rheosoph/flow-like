import { defineWidget } from "@flow-like/widget-sdk";

/** Filter state — mirrors `SalesFilter` in the package's Rust node */
export interface FilterValue {
	min: number;
	max: number;
	categories: string[];
}

interface Inputs {
	/** Panel headline @default "Filter revenue" */
	label: string;
	/** Lower revenue bound @minimum 0 @maximum 100000 @default 0 */
	min: number;
	/** Upper revenue bound; 0 means "no upper bound" @minimum 0 @maximum 100000 @default 0 */
	max: number;
	/** Categories the user can pick from @default [] */
	categories: string[];
	/** Categories selected on instantiate; empty keeps every category @default [] */
	selected: string[];
}

interface Events {
	/** The user applied the filter */
	applied: FilterValue;
	/** The user cleared the filter back to the instantiated defaults */
	// biome-ignore lint/suspicious/noConfusingVoidType: contract convention for payload-less events
	resetRequested: void;
}

interface Queries {
	/** Current filter state — feed straight into the Apply Sales Filter node */
	// biome-ignore lint/suspicious/noConfusingVoidType: contract convention for argument-less queries
	getValue: { args: void; returns: FilterValue };
	/** How many categories are currently selected */
	// biome-ignore lint/suspicious/noConfusingVoidType: contract convention for argument-less queries
	getCategoryCount: { args: void; returns: number };
}

export default defineWidget<Inputs, Events, Queries>({
	id: "filter-panel",
	name: "Filter Panel",
	description:
		"Revenue filter controls. The flow reads its state with the getValue query and can re-seed it at any time with Update Widget Inputs.",
	sizing: { defaultHeight: 260, resizable: true },
	dev: {
		fixtures: {
			blank: { categories: [], selected: [] },
			seeded: {
				label: "Filter Q3",
				min: 2000,
				max: 8000,
				categories: ["Hardware", "Software", "Services"],
				selected: ["Software"],
			},
		},
	},
});
