import { defineWidget } from "@flow-like/widget-sdk";

interface Inputs {
	/** Widget headline @default "Hello from Vanilla TS" */
	title: string;
	/** Counter start value @minimum 0 @maximum 100 @default 0 */
	count: number;
}

interface Events {
	/** Fired every time the counter button is clicked */
	increased: { value: number };
}

interface Queries {
	/** Current counter value */
	// biome-ignore lint/suspicious/noConfusingVoidType: contract convention for argument-less queries
	getCount: { args: void; returns: number };
}

export default defineWidget<Inputs, Events, Queries>({
	id: "hello-widget",
	name: "Hello Widget",
	description:
		"Minimal vanilla TypeScript widget: themed counter with a typed contract.",
	sizing: { defaultHeight: 240, resizable: true },
});
