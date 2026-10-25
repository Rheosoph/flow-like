import { describe, expect, test } from "bun:test";
import { type ILayer, ILayerType } from "../../lib/schema/flow/board";
import type { INode } from "../../lib/schema/flow/node";
import { type IPin, IPinType } from "../../lib/schema/flow/pin";
import { IValueType, IVariableType } from "../../lib/schema/flow/variable";
import {
	buildInitialEdits,
	pinNameForLabel,
	serializePinEdits,
} from "./pin-editor-model";

const pin = (
	id: string,
	name: string,
	friendly_name: string,
	pin_type: IPinType,
	data_type: IVariableType,
	index: number,
): IPin => ({
	id,
	name,
	friendly_name,
	pin_type,
	data_type,
	index,
	description: "",
	connected_to: [`${id}-wire`],
	depends_on: [],
	value_type: IValueType.Normal,
});

const pinMap = (...pins: IPin[]) =>
	Object.fromEntries(pins.map((p) => [p.id, p]));

const namesOf = (pins: Record<string, IPin>) =>
	Object.fromEntries(Object.values(pins).map((p) => [p.id, p.name]));

// FlowScript authors parameters with `name === friendly_name`, camelCase included.
const flowScriptFunction: INode = {
	id: "read-private-document",
	name: "events_generic",
	friendly_name: "readPrivateDocument",
	category: "Events",
	description: "",
	pins: pinMap(
		pin(
			"exec",
			"exec_out",
			"Exec Out",
			IPinType.Output,
			IVariableType.Execution,
			1,
		),
		pin(
			"payload",
			"payload",
			"Payload",
			IPinType.Output,
			IVariableType.Struct,
			2,
		),
		pin(
			"file-id",
			"fileId",
			"fileId",
			IPinType.Output,
			IVariableType.String,
			3,
		),
	),
};

const functionLayer = {
	id: "function-layer",
	name: "lookupUser",
	type: ILayerType.Function,
	comments: {},
	coordinates: [0, 0, 0],
	nodes: {},
	variables: {},
	pins: pinMap(
		pin("in", "exec_in", "Input", IPinType.Input, IVariableType.Execution, 1),
		pin("user-id", "userId", "userId", IPinType.Input, IVariableType.String, 2),
		pin("out", "exec_out", "Done", IPinType.Output, IVariableType.Execution, 1),
	),
} as ILayer;

describe("saving the pin editor without edits", () => {
	test("keeps every generic event parameter name", () => {
		const saved = serializePinEdits(
			flowScriptFunction.pins,
			buildInitialEdits(flowScriptFunction),
			true,
		);
		expect(namesOf(saved)).toEqual(namesOf(flowScriptFunction.pins));
	});

	test("keeps every function layer boundary pin name and wire", () => {
		const saved = serializePinEdits(
			functionLayer.pins,
			buildInitialEdits(functionLayer),
			false,
		);
		expect(namesOf(saved)).toEqual(namesOf(functionLayer.pins));
		expect(saved["user-id"].connected_to).toEqual(["user-id-wire"]);
	});
});

describe("pinNameForLabel", () => {
	const fileId = flowScriptFunction.pins["file-id"];

	test("derives a machine name when the label changes", () => {
		expect(pinNameForLabel(fileId, "Document Id")).toBe("document_id");
	});

	test("restores the stored name when the label is typed back", () => {
		expect(pinNameForLabel(fileId, "fileId")).toBe("fileId");
	});

	test("derives a machine name for a pin the entity does not have yet", () => {
		expect(pinNameForLabel(undefined, "New Pin")).toBe("new_pin");
	});
});
