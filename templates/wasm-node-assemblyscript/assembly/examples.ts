/**
 * Example Nodes Entry Point (Multi-Node Package)
 *
 * Uses NodePackage from the SDK to register all nodes.
 * NodePackage handles get_nodes() serialization and run() dispatch automatically.
 *
 * Build with: npx asc assembly/examples.ts --target release -o build/examples.wasm
 */

import { NodePackage } from "@flow-like/wasm-sdk-assemblyscript/assembly/index";

export {
	alloc,
	dealloc,
	get_abi_version,
} from "@flow-like/wasm-sdk-assemblyscript/assembly/index";

import {
	AndGateNode,
	CompareNode,
	GateNode,
	IfBranchNode,
	NotGateNode,
	OrGateNode,
} from "../examples/control_flow";
import {
	AddNode,
	ClampNode,
	DivideNode,
	MultiplyNode,
	SubtractNode,
} from "../examples/math_nodes";
import {
	ConcatNode,
	ContainsNode,
	LengthNode,
	LowercaseNode,
	ReplaceNode,
	TrimNode,
	UppercaseNode,
} from "../examples/string_nodes";

const pkg = new NodePackage();

// Math nodes
pkg.register(new AddNode());
pkg.register(new SubtractNode());
pkg.register(new MultiplyNode());
pkg.register(new DivideNode());
pkg.register(new ClampNode());

// String nodes
pkg.register(new UppercaseNode());
pkg.register(new LowercaseNode());
pkg.register(new TrimNode());
pkg.register(new LengthNode());
pkg.register(new ContainsNode());
pkg.register(new ReplaceNode());
pkg.register(new ConcatNode());

// Control flow nodes
pkg.register(new IfBranchNode());
pkg.register(new CompareNode());
pkg.register(new AndGateNode());
pkg.register(new OrGateNode());
pkg.register(new NotGateNode());
pkg.register(new GateNode());

export function get_nodes(): i64 {
	return pkg.getNodes();
}

export function run(ptr: i32, len: i32): i64 {
	return pkg.run(ptr, len);
}
