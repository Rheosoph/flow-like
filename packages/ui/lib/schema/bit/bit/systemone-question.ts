export type ISystemOneQuestion =
	| {
			instructions: unknown;
			criteria: { [property: string]: unknown };
			type: "choice";
	  }
	| { instructions: unknown; criteria: Array<unknown>; type: "score" }
	| {
			instructions: unknown;
			criteria?: ISystemOneNoulCriteria | null;
			type: "noul";
	  };

export type ISystemOneNoulCriteria = { true?: unknown; false?: unknown };
