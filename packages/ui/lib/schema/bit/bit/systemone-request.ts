import type { ISystemOneQuestion } from "./systemone-question";

export type ISystemOneRequest = {
	state: unknown;
	questions: { [property: string]: ISystemOneQuestion };
	images?: Array<string>;
};
