import type { INode } from "../schema/flow/node";
import { IVariableType } from "../schema/flow/node";
import { GraphBuilder } from "./test-fixtures";
import type { AutoLayoutInput } from "./types";

function namePins(node: INode, inputs: string[], outputs: string[]) {
	for (const [side, names] of [
		["in", inputs],
		["out", outputs],
	] as const) {
		for (const [index, name] of names.entries()) {
			const pin = node.pins[`${node.id}:${side}-${index}`];
			pin.name = name;
			pin.friendly_name = name;
			if (name === "Row" || name === "Column" || name === "Length")
				pin.data_type = IVariableType.Integer;
			if (name === "Found" || name === "Is Folder")
				pin.data_type = IVariableType.Boolean;
		}
	}
}

/** Excel reads share file and row inputs along an execution spine. */
export function excelPathFixture(
	edgePathType: AutoLayoutInput["edgePathType"] = "default",
): AutoLayoutInput {
	const graph = new GraphBuilder();
	const start = graph.exec("get-path", {
		execIn: false,
		start: true,
		dataOuts: 2,
	});
	start.friendly_name = "GetPath Start";
	namePins(start, [], ["Excel Path", "Row"]);
	const found = graph.exec("set-found", { dataIns: 2, dataOuts: 1 });
	found.friendly_name = "Set Found";
	namePins(found, ["Found", "Value"], ["New Value"]);
	for (const id of ["read-bs", "read-number", "read-year"]) {
		const node = graph.exec(id, { dataIns: 4, dataOuts: 3 });
		node.friendly_name = "Excel Read Cell";
		namePins(
			node,
			["File", "Sheet", "Row", "Column"],
			["File", "Value", "Found"],
		);
	}
	for (const id of ["format-number", "format-row"]) {
		const node = graph.exec(id, { dataIns: 2, dataOuts: 1 });
		node.friendly_name = "Call FormatNr";
		namePins(node, ["Value", "Length"], ["Out"]);
	}
	for (const id of ["substring-number", "substring-year"]) {
		const node = graph.pure(id, { dataIns: 3 });
		node.friendly_name = "Substring";
		namePins(node, ["String", "Start", "Length"], ["Substring"]);
	}
	for (const id of ["format-folder", "format-file"]) {
		const node = graph.pure(id, { dataIns: 4 });
		node.friendly_name = "Format String";
		namePins(node, ["Input", "BS", "nr", "yr"], ["Formatted"]);
	}
	for (const id of ["child-folder", "child-file"]) {
		const node = graph.pure(id, { dataIns: 2 });
		node.friendly_name = "Child";
		namePins(node, ["Path", "Child"], ["Path"]);
	}
	const root = graph.pure("project-root", { dataIns: 0 });
	root.friendly_name = "Project Root";
	namePins(root, [], ["Path"]);
	const folder = graph.exec("is-folder", { dataIns: 1, dataOuts: 1 });
	folder.friendly_name = "Is Folder";
	namePins(folder, ["Path"], ["Is Folder"]);
	const finish = graph.exec("open-file", { dataIns: 1 });
	finish.friendly_name = "Open File";
	namePins(finish, ["Path"], []);

	const spine = [
		"get-path",
		"set-found",
		"read-bs",
		"read-number",
		"format-number",
		"format-row",
		"read-year",
		"is-folder",
		"open-file",
	];
	for (let index = 1; index < spine.length; index++)
		graph.execLink(spine[index - 1], spine[index]);
	graph.dataLink("get-path", "read-bs", 0, 0);
	graph.dataLink("get-path", "read-bs", 1, 2);
	graph.dataLink("read-bs", "read-number", 0, 0);
	graph.dataLink("read-bs", "read-number", 1, 2);
	graph.dataLink("read-number", "format-number", 1, 0);
	graph.dataLink("read-number", "substring-number", 1, 0);
	graph.dataLink("substring-number", "format-row", 0, 0);
	graph.dataLink("read-number", "read-year", 0, 0);
	graph.dataLink("format-row", "read-year", 0, 2);
	graph.dataLink("read-year", "substring-year", 1, 0);
	for (const id of ["format-folder", "format-file"]) {
		graph.dataLink("read-number", id, 1, 1);
		graph.dataLink("format-number", id, 0, 2);
		graph.dataLink("substring-year", id, 0, 3);
	}
	graph.dataLink("format-folder", "child-folder", 0, 1);
	graph.dataLink("format-file", "child-file", 0, 1);
	graph.dataLink("project-root", "child-folder", 0, 0);
	graph.dataLink("project-root", "child-file", 0, 0);
	graph.dataLink("child-folder", "is-folder", 0, 0);
	graph.dataLink("child-file", "open-file", 0, 0);
	return graph.build({ edgePathType });
}
