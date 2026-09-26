import { IValueType } from "../../../lib";
import {
	type IPin,
	IPinType,
	IVariableType,
} from "../../../lib/schema/flow/pin";

export type PinEditorKind =
	| "label"
	| "projectUser"
	| "remoteProject"
	| "remoteDatabase"
	| "remoteEvent"
	| "ontology"
	| "ontologyObject"
	| "ontologyAction"
	| "remoteOntology"
	| "remoteOntologyObject"
	| "remoteOntologyAction"
	| "widget"
	| "taxCode"
	| "boolean"
	| "enum"
	| "bit"
	| "fn"
	| "var"
	| "element"
	| "plain";

export function resolvePinEditorKind(
	pin: IPin,
	nodeName?: string,
): PinEditorKind {
	if (pin.pin_type === IPinType.Output || pin.depends_on.length > 0)
		return "label";
	const plainString =
		pin.data_type === IVariableType.String &&
		pin.value_type === IValueType.Normal;
	if (plainString) {
		switch (pin.name) {
			case "_flow_user_sub":
				return "projectUser";
			case "_flow_remote_app_id":
				return "remoteProject";
			case "_flow_remote_database":
				return "remoteDatabase";
			case "_flow_remote_event":
				return "remoteEvent";
		}
	}
	if (pin.name === "_flow_remote_event_meta") return "label";
	if (plainString) {
		const localOntology =
			nodeName === "ontology_query_objects" ||
			nodeName === "ontology_action_request" ||
			nodeName === "ontology_action_input";
		if (localOntology && pin.name === "ontology_id") return "ontology";
		if (nodeName === "ontology_query_objects" && pin.name === "object_type")
			return "ontologyObject";
		if (
			(nodeName === "ontology_action_request" ||
				nodeName === "ontology_action_input") &&
			pin.name === "action_id"
		)
			return "ontologyAction";
		const remoteOntology =
			nodeName === "ontology_query_remote_objects" ||
			nodeName === "ontology_query_remote_children" ||
			nodeName === "ontology_action_request_remote";
		if (remoteOntology && pin.name === "binding_id") return "remoteOntology";
		if (
			(nodeName === "ontology_query_remote_objects" ||
				nodeName === "ontology_query_remote_children") &&
			pin.name === "object_type"
		)
			return "remoteOntologyObject";
		if (
			nodeName === "ontology_action_request_remote" &&
			pin.name === "action_id"
		)
			return "remoteOntologyAction";
		if (
			nodeName === "a2ui_instantiate_widget" &&
			pin.name === "widget_selector"
		)
			return "widget";
		if (nodeName === "request_payment" && pin.name === "product_tax_code")
			return "taxCode";
	}
	if (pin.data_type === IVariableType.Boolean) return "boolean";
	if (
		pin.data_type === IVariableType.String &&
		(pin.options?.valid_values?.length ?? 0) > 0
	)
		return "enum";
	if (plainString && pin.name.startsWith("bit_id")) return "bit";
	if (plainString && pin.name.startsWith("fn_ref")) return "fn";
	if (plainString && pin.name.startsWith("var_ref")) return "var";
	if (
		pin.name.startsWith("element_ref") &&
		pin.value_type === IValueType.Normal
	)
		return "element";
	return "plain";
}
