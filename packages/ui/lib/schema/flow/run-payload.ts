import type { IVariable } from "./variable";

export interface IRunPayload {
	id: string;
	payload?: any;
	runtime_variables?: Record<string, IVariable>;
	[property: string]: any;
}
