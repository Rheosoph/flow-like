export interface IChoiceOption {
	id: string;
	label: string;
	description?: string;
	freeform?: boolean;
}

export type IFormFieldType = "text" | "number" | "boolean" | "select";

export interface IFormField {
	id: string;
	label: string;
	description?: string;
	field_type: IFormFieldType;
	required?: boolean;
	default_value?: any;
	options?: IChoiceOption[];
}

export interface ISingleChoiceInteraction {
	type: "single_choice";
	options: IChoiceOption[];
	allow_freeform?: boolean;
}

export interface IMultipleChoiceInteraction {
	type: "multiple_choice";
	options: IChoiceOption[];
	min_selections?: number;
	max_selections?: number;
}

export interface IFormInteraction {
	type: "form";
	schema?: Record<string, any>;
	fields?: IFormField[];
}

export type IInteractionType =
	| ISingleChoiceInteraction
	| IMultipleChoiceInteraction
	| IFormInteraction;

export type IInteractionStatus =
	| "pending"
	| "responded"
	| "expired"
	| "cancelled";

export interface IInteractionRequest {
	id: string;
	name: string;
	description: string;
	interaction_type: IInteractionType;
	status: IInteractionStatus;
	ttl_seconds: number;
	expires_at: number;
	run_id?: string;
	app_id?: string;
	responder_jwt?: string;
	response_value?: any;
}

export interface IInteractionResponse {
	interaction_id: string;
	value: any;
}

export type IInteractionPollResult =
	| { status: "pending" }
	| { status: "responded"; value: any }
	| { status: "expired" }
	| { status: "cancelled" };
