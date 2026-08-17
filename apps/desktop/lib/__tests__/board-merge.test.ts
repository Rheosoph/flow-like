import type { IBoard, INode, IPin } from "@flow-like/flow-like-ui";
import { describe, expect, it } from "vitest";
import { mergeRemoteBoard } from "../../components/tauri-provider/board-merge";

const pin = (id: string, sensitive: boolean, value: number[] | null): IPin =>
	({
		id,
		name: id,
		friendly_name: id,
		description: "",
		pin_type: "Input",
		data_type: "String",
		value_type: "Normal",
		depends_on: [],
		connected_to: [],
		index: 0,
		default_value: value,
		options: sensitive ? { sensitive: true } : null,
	}) as unknown as IPin;

const node = (id: string, hash: number, pins: IPin[]): INode =>
	({
		id,
		name: "demo",
		friendly_name: "Demo",
		description: "",
		category: "",
		hash,
		pins: Object.fromEntries(pins.map((p) => [p.id, p])),
	}) as unknown as INode;

const board = (nodes: INode[], updated = 1): IBoard =>
	({
		id: "b",
		name: "Board",
		description: "",
		nodes: Object.fromEntries(nodes.map((n) => [n.id, n])),
		variables: {},
		comments: {},
		layers: {},
		refs: {},
		page_ids: [],
		version: [0, 0, 1],
		viewport: [0, 0, 1],
		created_at: { secs_since_epoch: 1, nanos_since_epoch: 0 },
		updated_at: { secs_since_epoch: updated, nanos_since_epoch: 0 },
	}) as unknown as IBoard;

describe("mergeRemoteBoard secret preservation", () => {
	it("keeps a local sensitive pin literal the server stripped, even when hashes match", () => {
		const secret = [0x22, 0x6b, 0x22];
		const local = board([node("n", 42, [pin("api_key", true, secret)])]);
		// Same runtime hash: the server hashed the node with the value before stripping it.
		const remote = board([node("n", 42, [pin("api_key", true, null)])], 2);

		const merged = mergeRemoteBoard(remote, local);

		expect(merged.nodes.n.pins.api_key.default_value).toEqual(secret);
	});

	it("does not resurrect a value the server actually cleared on a non-sensitive pin", () => {
		const local = board([node("n", 1, [pin("plain", false, [0x31])])]);
		const remote = board([node("n", 2, [pin("plain", false, null)])], 2);

		const merged = mergeRemoteBoard(remote, local);

		expect(merged.nodes.n.pins.plain.default_value).toBeNull();
	});

	it("still takes a remote sensitive value when the server sent one", () => {
		const local = board([node("n", 1, [pin("api_key", true, [0x31])])]);
		const remote = board([node("n", 2, [pin("api_key", true, [0x32])])], 2);

		const merged = mergeRemoteBoard(remote, local);

		expect(merged.nodes.n.pins.api_key.default_value).toEqual([0x32]);
	});
});
