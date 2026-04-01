import { Context } from "./context";
import { parseInput, serializeDefinition, serializeResult } from "./host";
import { ExecutionResult, type NodeDefinition, PackageNodes } from "./types";

export abstract class FlowNode {
	abstract define(): NodeDefinition;
	abstract execute(ctx: Context): ExecutionResult;
}

export class NodePackage {
	private _nodes: FlowNode[] = [];

	register(node: FlowNode): NodePackage {
		this._nodes.push(node);
		return this;
	}

	getNodes(): i64 {
		const pkg = new PackageNodes();
		for (let i = 0; i < this._nodes.length; i++) {
			pkg.addNode(this._nodes[i].define());
		}
		return pkg.toWasm();
	}

	run(ptr: i32, len: i32): i64 {
		const input = parseInput(ptr, len);
		const nodeName = input.node_name;

		for (let i = 0; i < this._nodes.length; i++) {
			const node = this._nodes[i];
			if (node.define().name == nodeName) {
				const ctx = new Context(input);
				return serializeResult(node.execute(ctx));
			}
		}

		return serializeResult(ExecutionResult.fail("Unknown node: " + nodeName));
	}
}

export function singleNode(node: FlowNode): i64 {
	return serializeDefinition(node.define());
}

export function runSingle(node: FlowNode, ptr: i32, len: i32): i64 {
	const input = parseInput(ptr, len);
	const ctx = new Context(input);
	return serializeResult(node.execute(ctx));
}
