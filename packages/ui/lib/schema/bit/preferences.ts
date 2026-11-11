export interface IPreferences {
	coding_weight?: number | null;
	cost_weight?: number | null;
	creativity_weight?: number | null;
	factuality_weight?: number | null;
	function_calling_weight?: number | null;
	model_hint?: null | string;
	multilinguality_weight?: number | null;
	multimodal?: boolean | null;
	openness_weight?: number | null;
	reasoning_weight?: number | null;
	safety_weight?: number | null;
	speed_weight?: number | null;
	[property: string]: any;
}
