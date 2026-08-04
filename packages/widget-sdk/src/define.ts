import type { WidgetSizing } from "./contract";

export type WidgetInputsShape = object;
export type WidgetEventsShape = object;
export type WidgetQueriesShape = object;

export type QueryArgs<Q, Name extends keyof Q> = Q[Name] extends {
	args: infer A;
}
	? A
	: unknown;

export type QueryReturns<Q, Name extends keyof Q> = Q[Name] extends {
	returns: infer R;
}
	? R
	: unknown;

export interface WidgetDevConfig<Inputs extends WidgetInputsShape> {
	fixtures?: Record<string, Partial<Inputs>>;
}

export interface WidgetConfig<Inputs extends WidgetInputsShape> {
	id: string;
	name: string;
	description: string;
	sizing?: WidgetSizing;
	dev?: WidgetDevConfig<Inputs>;
}

export interface WidgetDefinition<
	Inputs extends WidgetInputsShape = Record<string, unknown>,
	Events extends WidgetEventsShape = Record<string, unknown>,
	Queries extends WidgetQueriesShape = Record<
		string,
		{ args: unknown; returns: unknown }
	>,
> extends WidgetConfig<Inputs> {
	// Phantom marker so the Events/Queries type arguments survive structural
	// typing; never present at runtime.
	readonly __types?: {
		inputs: Inputs;
		events: Events;
		queries: Queries;
	};
}

export type WidgetInputsOf<D> = D extends WidgetDefinition<
	infer Inputs,
	WidgetEventsShape,
	WidgetQueriesShape
>
	? Inputs
	: never;

export type WidgetEventsOf<D> = D extends WidgetDefinition<
	WidgetInputsShape,
	infer Events,
	WidgetQueriesShape
>
	? Events
	: never;

export type WidgetQueriesOf<D> = D extends WidgetDefinition<
	WidgetInputsShape,
	WidgetEventsShape,
	infer Queries
>
	? Queries
	: never;

export function defineWidget<
	Inputs extends WidgetInputsShape = Record<string, unknown>,
	Events extends WidgetEventsShape = Record<string, unknown>,
	Queries extends WidgetQueriesShape = Record<
		string,
		{ args: unknown; returns: unknown }
	>,
>(config: WidgetConfig<Inputs>): WidgetDefinition<Inputs, Events, Queries> {
	return config;
}
