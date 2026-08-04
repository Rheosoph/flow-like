import { describe, expect, test } from "bun:test";
import type { IBoard } from "./schema/flow/board";
import { ICommandType } from "./schema/flow/board/commands/generic-command";
import { buildTemplateCopyPasteCommand } from "./template-copy-paste";

describe("buildTemplateCopyPasteCommand", () => {
	test("includes the schema ref table with the copied graph", () => {
		const template = {
			nodes: { node: { id: "node" } },
			comments: {},
			layers: {},
			variables: {},
			refs: {
				"schema-ref": '{"type":"object"}',
				"__flow_like_internal_v1/private": "must-not-leak",
			},
		} as unknown as IBoard;

		const command = buildTemplateCopyPasteCommand(template, "layer");

		expect(command.command_type).toBe(ICommandType.CopyPaste);
		expect(command.original_nodes.map((node) => node.id)).toEqual(["node"]);
		expect(command.original_refs).toEqual({
			"schema-ref": '{"type":"object"}',
		});
		expect(command.current_layer).toBe("layer");
	});
});
