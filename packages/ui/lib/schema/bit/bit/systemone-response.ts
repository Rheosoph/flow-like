export type ISystemOneResponse = {
	model: string;
	answers: { [property: string]: ISystemOneAnswer };
	usage?: ISystemOneUsage | null;
	[property: string]: unknown;
};

export type ISystemOneAnswer =
	| {
			choice: string;
			probabilities: { [property: string]: number };
			confidence: number;
			type: "choice";
			[property: string]: unknown;
	  }
	| {
			score: number;
			legend: { [property: string]: unknown };
			probabilities: { [property: string]: number };
			confidence: number;
			type: "score";
			[property: string]: unknown;
	  }
	| { noul: number; type: "noul"; [property: string]: unknown };

export type ISystemOneUsage = {
	input_tokens: number;
	output_tokens: number;
	[property: string]: unknown;
};
