import { describe, expect, test } from "bun:test";
import { readFileSync, readdirSync } from "node:fs";
import { resolve } from "node:path";
import type { IBoard, INode, IPin } from "../schema";
import { IPinType, IVariableType } from "../schema";
import { ILayerType } from "../schema/flow/board";
import { parseBpmn, walkFlowNodes, walkSequenceFlows } from "./bpmn-model";
import { BPMN_NODE_SPECS } from "./bpmn-nodes";
import { translateBpmn } from "./bpmn-translator";
import { detectFormat } from "./detect";
import { bpmnCatalogFixture } from "./fixtures/catalog";
import type { TranslationResult } from "./types";

const FIXTURES = resolve(import.meta.dir, "fixtures", "bpmn");

function fixture(name: string): string {
	return readFileSync(resolve(FIXTURES, name), "utf-8");
}

function translate(name: string): TranslationResult {
	return translateBpmn(parseBpmn(fixture(name)));
}

function nodesNamed(board: IBoard, name: string): INode[] {
	return Object.values(board.nodes).filter((n) => n.name === name);
}

function byComment(board: IBoard, needle: string): INode | undefined {
	return Object.values(board.nodes).find((n) => n.comment?.includes(needle));
}

function pin(node: INode, name: string, type: IPinType): IPin {
	const found = Object.values(node.pins).find(
		(p) => p.name === name && p.pin_type === type,
	);
	if (!found) throw new Error(`${node.name} has no ${type} pin ${name}`);
	return found;
}

function allPins(board: IBoard): Map<string, { pin: IPin; owner: string }> {
	const map = new Map<string, { pin: IPin; owner: string }>();
	for (const node of Object.values(board.nodes)) {
		for (const p of Object.values(node.pins))
			map.set(p.id, { pin: p, owner: node.id });
	}
	for (const layer of Object.values(board.layers)) {
		for (const p of Object.values(layer.pins))
			map.set(p.id, { pin: p, owner: layer.id });
	}
	return map;
}

function layerNamed(board: IBoard, name: string) {
	return Object.values(board.layers).find((l) => l.name === name);
}

/** Follows exec wiring from a pin to the node that receives it. */
function execTarget(board: IBoard, from: IPin): INode | undefined {
	const id = from.connected_to[0];
	if (!id) return undefined;
	return Object.values(board.nodes).find((n) => n.pins[id]);
}

/**
 * The invariants every fragment must meet before the backend will accept it
 * unchanged: symmetric edges, single-target exec outputs, known node names,
 * nodes in `board.nodes` with a layer that exists, unique pin indices.
 */
function assertFragmentInvariants(board: IBoard): void {
	const pins = allPins(board);
	for (const [id, { pin: p }] of pins) {
		for (const target of p.connected_to) {
			const other = pins.get(target);
			expect(
				other,
				`dangling connected_to ${target} from ${p.name}`,
			).toBeDefined();
			expect(other?.pin.depends_on).toContain(id);
		}
		for (const source of p.depends_on) {
			const other = pins.get(source);
			expect(other, `dangling depends_on ${source} on ${p.name}`).toBeDefined();
			expect(other?.pin.connected_to).toContain(id);
		}
		if (
			p.data_type === IVariableType.Execution &&
			p.pin_type === IPinType.Output
		) {
			expect(
				p.connected_to.length,
				`exec output ${p.name} fans out`,
			).toBeLessThanOrEqual(1);
		}
	}
	for (const node of Object.values(board.nodes)) {
		expect(
			BPMN_NODE_SPECS[node.name],
			`unknown node ${node.name}`,
		).toBeDefined();
		expect(node.name).not.toBe("todo_placeholder");
		if (node.layer) expect(board.layers[node.layer]).toBeDefined();
		for (const type of [IPinType.Input, IPinType.Output]) {
			const indices = Object.values(node.pins)
				.filter((p) => p.pin_type === type)
				.map((p) => p.index);
			expect(new Set(indices).size).toBe(indices.length);
		}
	}
	for (const layer of Object.values(board.layers)) {
		if (layer.parent_id) expect(board.layers[layer.parent_id]).toBeDefined();
		expect(layer.in_coordinates).toBeNull();
		expect(layer.out_coordinates).toBeNull();
		expect(Object.keys(layer.nodes)).toHaveLength(0);
	}
	for (const comment of Object.values(board.comments)) {
		if (comment.layer) expect(board.layers[comment.layer]).toBeDefined();
	}
}

describe("detectFormat", () => {
	test("recognises BPMN XML and rejects other XML", () => {
		const detection = detectFormat(fixture("simple-linear.bpmn"));
		expect(detection.format).toBe("bpmn");
		expect(detection.parsed).toBeDefined();
		expect(detectFormat("<svg xmlns='x'/>").format).toBe("unknown");
		expect(detectFormat("<svg xmlns='x'/>").error).toMatch(
			/root <svg> is not a BPMN definitions element/,
		);
		expect(detectFormat("<a><b></a>").error).toMatch(/Invalid XML/);
		expect(detectFormat("not a file at all").format).toBe("unknown");
	});
});

describe("simple linear process", () => {
	const result = translate("simple-linear.bpmn");
	const { board } = result;

	test("meets the fragment invariants", () => {
		assertFragmentInvariants(board);
		expect(result.format).toBe("bpmn");
		expect(board.name).toBe("Simple Linear");
	});

	test("start event becomes a start node that keeps its BPMN name", () => {
		const [start] = nodesNamed(board, "events_simple");
		expect(start).toBeDefined();
		expect(start.start).toBe(true);
		expect(start.friendly_name).toBe("Order received");
		expect(start.layer).toBeNull();
	});

	test("abstract and service tasks become placeholder layers chained in order", () => {
		const check = layerNamed(board, "Check order");
		const notify = layerNamed(board, "Notify warehouse");
		expect(check?.type).toBe(ILayerType.Collapsed);
		expect(notify?.comment).toContain(
			"zeebe:taskDefinition type=notify-warehouse",
		);
		expect(notify?.comment).toContain("input id = =orderId");

		const [start] = nodesNamed(board, "events_simple");
		const checkInner = execTarget(
			board,
			pin(start, "exec_out", IPinType.Output),
		);
		expect(checkInner?.name).toBe("log_warning");
		expect(checkInner?.layer).toBe(check?.id);
		const notifyInner = execTarget(
			board,
			pin(checkInner as INode, "exec_out", IPinType.Output),
		);
		expect(notifyInner?.layer).toBe(notify?.id);
		const end = execTarget(
			board,
			pin(notifyInner as INode, "exec_out", IPinType.Output),
		);
		expect(end?.name).toBe("log_info");
		expect(end?.comment).toContain("Order handled");
	});

	test("positions come from the diagram, scaled and normalised", () => {
		const [start] = nodesNamed(board, "events_simple");
		expect(start.coordinates?.[0]).toBe(0);
		const end = byComment(board, "Order handled");
		expect((end?.coordinates?.[0] ?? 0) > (start.coordinates?.[0] ?? 0)).toBe(
			true,
		);
	});

	test("reports placeholders in stats and status", () => {
		expect(result.stats.todo).toBe(2);
		expect(result.stats.totalNodes).toBe(4);
		expect(result.status).toBe("partial");
		expect(result.diagnostics.filter((d) => d.level === "error")).toHaveLength(
			0,
		);
	});
});

describe("gateways", () => {
	const result = translate("gateways.bpmn");
	const { board } = result;

	test("meets the fragment invariants", () => {
		assertFragmentInvariants(board);
	});

	test("exclusive split becomes a branch chain with conditions in comments and the default on the last false", () => {
		const branches = nodesNamed(board, "control_branch");
		expect(branches).toHaveLength(2);
		const big = branches.find((b) => b.comment?.includes("${amount > 1000}"));
		const mid = branches.find((b) => b.comment?.includes("amount <= 1000"));
		expect(big).toBeDefined();
		expect(mid).toBeDefined();
		expect(
			execTarget(board, pin(big as INode, "false", IPinType.Output))?.id,
		).toBe(mid?.id);
		const approve = execTarget(
			board,
			pin(big as INode, "true", IPinType.Output),
		);
		expect(approve?.layer).toBe(layerNamed(board, "Approve")?.id);
		// default flow → Xor_join (pass-through) → And_split (Parallel Execution)
		const afterDefault = execTarget(
			board,
			pin(mid as INode, "false", IPinType.Output),
		);
		expect(afterDefault?.name).toBe("control_par_execution");
	});

	test("parallel split and join use Parallel Execution and Gather", () => {
		const [fork] = nodesNamed(board, "control_par_execution");
		const [gather] = nodesNamed(board, "control_gather");
		expect(fork).toBeDefined();
		expect(gather).toBeDefined();
		const outs = Object.values(fork.pins).filter(
			(p) => p.name === "exec_out" && p.pin_type === IPinType.Output,
		);
		expect(outs.filter((p) => p.connected_to.length === 1)).toHaveLength(2);
		const ins = Object.values(gather.pins).filter(
			(p) => p.name === "exec_in" && p.pin_type === IPinType.Input,
		);
		expect(ins.filter((p) => p.depends_on.length === 1)).toHaveLength(2);
		const done = execTarget(board, pin(gather, "exec_done", IPinType.Output));
		expect(done?.name).toBe("log_info");
	});

	test("an exclusive merge needs no node: both user tasks feed the fork", () => {
		const [fork] = nodesNamed(board, "control_par_execution");
		const forkIn = pin(fork, "exec_in", IPinType.Input);
		expect(forkIn.depends_on.length).toBe(3);
	});

	test("vendor attributes are preserved on placeholders", () => {
		expect(layerNamed(board, "Approve")?.comment).toContain(
			"camunda:assignee = manager",
		);
		expect(layerNamed(board, "Ship goods")?.comment).toContain(
			"camunda:topic = shipping",
		);
	});
});

describe("sub-process, boundaries and call activity", () => {
	const result = translate("subprocess-boundary.bpmn");
	const { board } = result;

	test("meets the fragment invariants", () => {
		assertFragmentInvariants(board);
	});

	test("embedded sub-process is a nested layer entered through its start event", () => {
		const sub = layerNamed(board, "Collect payment");
		expect(sub?.type).toBe(ILayerType.Collapsed);
		expect(sub?.parent_id).toBeNull();
		const charge = layerNamed(board, "Charge card");
		expect(charge?.parent_id).toBe(sub?.id);
	});

	test("error end inside the sub-process routes to the error boundary marker", () => {
		const logError = nodesNamed(board, "log_error")[0];
		expect(logError).toBeDefined();
		const marker = execTarget(
			board,
			pin(logError, "exec_out", IPinType.Output),
		);
		expect(marker?.name).toBe("log_info");
		expect(marker?.comment).toContain("boundary event");
		const failed = execTarget(
			board,
			pin(marker as INode, "exec_out", IPinType.Output),
		);
		expect(failed?.name).toBe("log_warning");
		expect(failed?.comment).toContain("Terminate end event");
	});

	test("timer boundary path leads into the reminder placeholder", () => {
		const delay = nodesNamed(board, "delay")[0];
		const marker = execTarget(board, pin(delay, "exec_out", IPinType.Output));
		const remind = execTarget(
			board,
			pin(marker as INode, "exec_out", IPinType.Output),
		);
		expect(remind?.layer).toBe(layerNamed(board, "Send reminder")?.id);
	});

	test("non-interrupting timer cycle forks a delay next to the sub-process", () => {
		const delay = nodesNamed(board, "delay")[0];
		expect(delay).toBeDefined();
		const timeDefault = JSON.parse(
			new TextDecoder().decode(
				Uint8Array.from(pin(delay, "time", IPinType.Input).default_value ?? []),
			),
		);
		expect(timeDefault).toBe(3_600_000);
		const fork = nodesNamed(board, "control_par_execution").find((n) =>
			n.comment?.includes("Non-interrupting timer"),
		);
		expect(fork).toBeDefined();
	});

	test("terminate end event logs a warning and is reported", () => {
		const terminate = nodesNamed(board, "log_warning").find((n) =>
			n.comment?.includes("Terminate end event"),
		);
		expect(terminate).toBeDefined();
		expect(
			result.diagnostics.some((d) => d.message.includes("Terminate end event")),
		).toBe(true);
	});

	test("call activity becomes a Call Function into a Function layer built from the called process", () => {
		const fn = Object.values(board.layers).find(
			(l) => l.type === ILayerType.Function,
		);
		expect(fn?.name).toBe("Archive");
		const call = nodesNamed(board, "control_call_function")[0];
		expect(call).toBeDefined();
		const ref = JSON.parse(
			new TextDecoder().decode(
				Uint8Array.from(
					pin(call, "function_layer_id", IPinType.Input).default_value ?? [],
				),
			),
		);
		expect(ref).toBe(fn?.id);
		expect(
			pin(call, "exec_in", IPinType.Input).depends_on.length,
		).toBeGreaterThan(0);
		const done = execTarget(board, pin(call, "exec_out", IPinType.Output));
		expect(done?.comment).toContain("Done");

		const fnIn = Object.values(fn?.pins ?? {}).find(
			(p) => p.pin_type === IPinType.Input,
		);
		const fnOut = Object.values(fn?.pins ?? {}).find(
			(p) => p.pin_type === IPinType.Output,
		);
		expect(fnIn?.connected_to).toHaveLength(1);
		expect(fnOut?.depends_on.length).toBeGreaterThan(0);
		const first = execTarget(board, fnIn as IPin);
		expect(first?.layer).toBe(layerNamed(board, "Compute key")?.id);
		expect(
			nodesNamed(board, "events_simple").every((n) => n.layer !== fn?.id),
		).toBe(true);
	});

	test("event sub-process keeps its own start event as an independent entry", () => {
		const eventSub = layerNamed(board, "On cancel");
		expect(eventSub).toBeDefined();
		const start = nodesNamed(board, "events_generic").find(
			(n) => n.layer === eventSub?.id,
		);
		expect(start?.start).toBe(true);
		expect(start?.friendly_name).toBe("Cancel requested");
	});
});

describe("collaboration", () => {
	const result = translate("collaboration.bpmn");
	const { board } = result;

	test("meets the fragment invariants", () => {
		assertFragmentInvariants(board);
	});

	test("each pool with a process becomes its own layer", () => {
		const pools = Object.values(board.layers).filter(
			(l) => l.parent_id === null,
		);
		expect(pools.length).toBeGreaterThanOrEqual(2);
		expect(board.name).toBeDefined();
	});

	test("data objects become variables read and written next to their activities", () => {
		expect(Object.keys(board.variables).length).toBeGreaterThan(0);
		expect(result.stats.variables).toBe(Object.keys(board.variables).length);
		const sets = nodesNamed(board, "variable_set");
		const gets = nodesNamed(board, "variable_get");
		expect(sets.length + gets.length).toBeGreaterThan(0);
		for (const node of [...sets, ...gets]) {
			const ref = JSON.parse(
				new TextDecoder().decode(
					Uint8Array.from(
						pin(node, "var_ref", IPinType.Input).default_value ?? [],
					),
				),
			);
			expect(board.variables[ref]).toBeDefined();
		}
	});

	test("message flows, lanes and annotations become comments", () => {
		const comments = Object.values(board.comments);
		expect(
			comments.some((c) => c.content.startsWith("Message flows between pools")),
		).toBe(true);
		expect(comments.some((c) => (c.z_index ?? 0) < 0)).toBe(true);
	});

	test("multi-instance activities are wrapped in a loop node", () => {
		const loops = [
			...nodesNamed(board, "control_for_each"),
			...nodesNamed(board, "control_par_for_each"),
		];
		expect(loops.length).toBeGreaterThan(0);
		const loop = loops[0];
		const body = execTarget(board, pin(loop, "exec_out", IPinType.Output));
		expect(body).toBeDefined();
	});

	test("every BPMN flow node is accounted for", () => {
		const defs = parseBpmn(fixture("collaboration.bpmn"));
		let expected = 0;
		for (const process of defs.processes) {
			const count = (container: typeof process): number =>
				container.flowNodes.reduce(
					(sum, n) => sum + 1 + (n.body ? count(n.body as typeof process) : 0),
					0,
				);
			expected += count(process);
		}
		expect(result.stats.totalNodes).toBe(expected);
	});
});

describe("with a live catalog", () => {
	const catalog = bpmnCatalogFixture();
	const result = translateBpmn(parseBpmn(fixture("gateways.bpmn")), catalog);
	const { board } = result;

	test("clones catalog nodes so version, pin names and descriptions come from the catalog", () => {
		assertFragmentInvariants(board);
		for (const node of Object.values(board.nodes)) {
			const template = catalog.find((c) => c.name === node.name);
			expect(node.version).toBe(template?.version ?? null);
			expect(node.description).toBe(`Catalog ${node.name}`);
			const templateIds = new Set(Object.keys(template?.pins ?? {}));
			for (const p of Object.values(node.pins)) {
				expect(templateIds.has(p.id)).toBe(false);
				if (p.name !== "exec_out" && p.name !== "exec_in") {
					expect(p.description).toBe(`catalog pin ${p.name}`);
				}
			}
		}
	});

	test("grows repeatable pins beyond the catalog's two", () => {
		const fork = nodesNamed(board, "control_par_execution")[0];
		const outs = Object.values(fork.pins).filter(
			(p) => p.name === "exec_out" && p.pin_type === IPinType.Output,
		);
		expect(outs.length).toBeGreaterThanOrEqual(2);
		expect(new Set(outs.map((p) => p.index)).size).toBe(outs.length);
	});

	test("a catalog missing a composition node falls back to a placeholder", () => {
		const withoutHttp = bpmnCatalogFixture([
			"http_make_request",
			"http_fetch",
			"http_response_to_json",
		]);
		const xml = fixture("collaboration.bpmn");
		const partial = translateBpmn(parseBpmn(xml), withoutHttp);
		expect(nodesNamed(partial.board, "http_fetch")).toHaveLength(0);
		expect(partial.stats.todo).toBeGreaterThan(result.stats.todo);
	});
});

/** A minimal process body wrapped in a default-namespace document with no diagram. */
function process(body: string, extra = ""): TranslationResult {
	return translateBpmn(
		parseBpmn(
			`<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">${extra}<process id="P" name="Probe" isExecutable="true">${body}</process></definitions>`,
		),
		bpmnCatalogFixture(),
	);
}

function execOuts(node: INode, name: string): IPin[] {
	return Object.values(node.pins)
		.filter((p) => p.name === name && p.pin_type === IPinType.Output)
		.sort((a, b) => a.index - b.index);
}

describe("gateway and split semantics", () => {
	test("a gateway that both joins and splits keeps the join and the split", () => {
		const { board } = process(`
			<startEvent id="s1"/><startEvent id="s2"/>
			<sequenceFlow id="f1" sourceRef="s1" targetRef="g"/>
			<sequenceFlow id="f2" sourceRef="s2" targetRef="g"/>
			<parallelGateway id="g"/>
			<sequenceFlow id="f3" sourceRef="g" targetRef="a"/>
			<sequenceFlow id="f4" sourceRef="g" targetRef="b"/>
			<task id="a"/><task id="b"/>`);
		assertFragmentInvariants(board);
		const gather = nodesNamed(board, "control_gather")[0];
		const fork = nodesNamed(board, "control_par_execution")[0];
		expect(gather).toBeDefined();
		expect(fork).toBeDefined();
		// Both starts arrive on their own Gather input, and its completion drives
		// the fork that feeds both branches.
		const ins = Object.values(gather.pins).filter(
			(p) => p.name === "exec_in" && p.pin_type === IPinType.Input,
		);
		expect(ins.filter((p) => p.depends_on.length === 1)).toHaveLength(2);
		expect(
			execTarget(board, pin(gather, "exec_done", IPinType.Output))?.id,
		).toBe(fork.id);
		expect(
			execOuts(fork, "exec_out").filter((p) => p.connected_to.length === 1),
		).toHaveLength(2);
	});

	test("an exclusive split with three conditions and no default chains three branches and warns", () => {
		const result = process(`
			<startEvent id="s"/>
			<sequenceFlow id="f0" sourceRef="s" targetRef="g"/>
			<exclusiveGateway id="g"/>
			<sequenceFlow id="fa" sourceRef="g" targetRef="a"><conditionExpression>\${x==1}</conditionExpression></sequenceFlow>
			<sequenceFlow id="fb" sourceRef="g" targetRef="b"><conditionExpression>\${x==2}</conditionExpression></sequenceFlow>
			<sequenceFlow id="fc" sourceRef="g" targetRef="c"><conditionExpression>\${x==3}</conditionExpression></sequenceFlow>
			<task id="a"/><task id="b"/><task id="c"/>`);
		assertFragmentInvariants(result.board);
		const branches = nodesNamed(result.board, "control_branch");
		expect(branches).toHaveLength(3);
		// Each false output falls through to the next test, and the last one is a
		// dead end because BPMN named no default.
		const chained = branches.filter(
			(b) => pin(b, "false", IPinType.Output).connected_to.length === 1,
		);
		expect(chained).toHaveLength(2);
		expect(
			result.diagnostics.some((d) => d.message.includes("no default flow")),
		).toBe(true);
	});

	test("an activity with two unconditioned outgoing flows forks", () => {
		const { board } = process(`
			<startEvent id="s"/>
			<sequenceFlow id="f0" sourceRef="s" targetRef="a"/>
			<task id="a"/>
			<sequenceFlow id="f1" sourceRef="a" targetRef="b"/>
			<sequenceFlow id="f2" sourceRef="a" targetRef="c"/>
			<task id="b"/><task id="c"/>`);
		assertFragmentInvariants(board);
		const fork = nodesNamed(board, "control_par_execution")[0];
		expect(fork).toBeDefined();
		expect(
			execOuts(fork, "exec_out").filter((p) => p.connected_to.length === 1),
		).toHaveLength(2);
	});

	test("an event-based gateway starts every branch and says so", () => {
		const result = process(`
			<startEvent id="s"/>
			<sequenceFlow id="f0" sourceRef="s" targetRef="g"/>
			<eventBasedGateway id="g"/>
			<sequenceFlow id="f1" sourceRef="g" targetRef="t"/>
			<sequenceFlow id="f2" sourceRef="g" targetRef="m"/>
			<intermediateCatchEvent id="t"><timerEventDefinition><timeDuration>PT5M</timeDuration></timerEventDefinition></intermediateCatchEvent>
			<intermediateCatchEvent id="m"><messageEventDefinition/></intermediateCatchEvent>`);
		assertFragmentInvariants(result.board);
		expect(nodesNamed(result.board, "control_par_execution")).toHaveLength(1);
		expect(
			result.diagnostics.some((d) =>
				d.message.includes("does not cancel the others"),
			),
		).toBe(true);
	});
});

describe("structural translation", () => {
	test("ranks nodes left to right when the file has no diagram", () => {
		const { board } = process(`
			<startEvent id="s"/>
			<sequenceFlow id="f1" sourceRef="s" targetRef="a"/>
			<task id="a"/>
			<sequenceFlow id="f2" sourceRef="a" targetRef="e"/>
			<endEvent id="e"/>`);
		const start = nodesNamed(board, "events_simple")[0];
		const end = byComment(board, "End Event");
		expect(start.coordinates?.[0]).toBe(0);
		expect(end?.coordinates?.[0]).toBeGreaterThan(0);
	});

	test("nests sub-processes two levels deep and enters each at its start event", () => {
		const { board } = process(`
			<startEvent id="s"/>
			<sequenceFlow id="f0" sourceRef="s" targetRef="outer"/>
			<subProcess id="outer" name="Outer">
				<startEvent id="os"/>
				<sequenceFlow id="f1" sourceRef="os" targetRef="inner"/>
				<subProcess id="inner" name="Inner">
					<startEvent id="is"/>
					<sequenceFlow id="f2" sourceRef="is" targetRef="deep"/>
					<task id="deep" name="Deep"/>
					<sequenceFlow id="f3" sourceRef="deep" targetRef="ie"/>
					<endEvent id="ie"/>
				</subProcess>
				<sequenceFlow id="f4" sourceRef="inner" targetRef="oe"/>
				<endEvent id="oe"/>
			</subProcess>`);
		assertFragmentInvariants(board);
		const outer = layerNamed(board, "Outer");
		const inner = layerNamed(board, "Inner");
		const deep = layerNamed(board, "Deep");
		expect(outer?.parent_id).toBeNull();
		expect(inner?.parent_id).toBe(outer?.id);
		expect(deep?.parent_id).toBe(inner?.id);
		// Start → the deep placeholder, with no node in between.
		const start = nodesNamed(board, "events_simple")[0];
		const first = execTarget(board, pin(start, "exec_out", IPinType.Output));
		expect(first?.layer).toBe(deep?.id);
	});

	test("a link throw continues at its paired catch", () => {
		const { board } = process(`
			<startEvent id="s"/>
			<sequenceFlow id="f0" sourceRef="s" targetRef="throw"/>
			<intermediateThrowEvent id="throw"><linkEventDefinition name="hop"/></intermediateThrowEvent>
			<intermediateCatchEvent id="catch"><linkEventDefinition name="hop"/></intermediateCatchEvent>
			<sequenceFlow id="f1" sourceRef="catch" targetRef="a"/>
			<task id="a" name="After"/>`);
		assertFragmentInvariants(board);
		const start = nodesNamed(board, "events_simple")[0];
		const after = execTarget(board, pin(start, "exec_out", IPinType.Output));
		expect(after?.layer).toBe(layerNamed(board, "After")?.id);
	});

	test("a call activity chain becomes nested function layers", () => {
		const result = translateBpmn(
			parseBpmn(`<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL">
				<process id="Main" isExecutable="true">
					<startEvent id="s"/>
					<sequenceFlow id="f0" sourceRef="s" targetRef="c1"/>
					<callActivity id="c1" name="First" calledElement="Second"/>
				</process>
				<process id="Second" name="Second">
					<startEvent id="s2"/>
					<sequenceFlow id="f1" sourceRef="s2" targetRef="c2"/>
					<callActivity id="c2" name="Nested" calledElement="Third"/>
				</process>
				<process id="Third" name="Third">
					<startEvent id="s3"/>
					<sequenceFlow id="f2" sourceRef="s3" targetRef="t"/>
					<task id="t" name="Work"/>
				</process>
			</definitions>`),
			bpmnCatalogFixture(),
		);
		assertFragmentInvariants(result.board);
		const functions = Object.values(result.board.layers).filter(
			(l) => l.type === ILayerType.Function,
		);
		expect(functions.map((l) => l.name).sort()).toEqual(["Second", "Third"]);
		expect(nodesNamed(result.board, "control_call_function")).toHaveLength(2);
		// Only the root process keeps a runnable entry point.
		expect(nodesNamed(result.board, "events_simple")).toHaveLength(1);
	});

	test("one data object shared by two activities is one variable", () => {
		const { board } = process(`
			<startEvent id="s"/>
			<sequenceFlow id="f0" sourceRef="s" targetRef="a"/>
			<task id="a" name="A">
				<dataOutputAssociation><targetRef>ref</targetRef></dataOutputAssociation>
			</task>
			<sequenceFlow id="f1" sourceRef="a" targetRef="b"/>
			<task id="b" name="B">
				<dataInputAssociation><sourceRef>ref</sourceRef></dataInputAssociation>
			</task>
			<dataObject id="obj"/>
			<dataObjectReference id="ref" name="Order" dataObjectRef="obj"/>`);
		assertFragmentInvariants(board);
		const variables = Object.values(board.variables);
		expect(variables).toHaveLength(1);
		expect(variables[0].name).toBe("Order");
		const refs = [
			...nodesNamed(board, "variable_get"),
			...nodesNamed(board, "variable_set"),
		].map((node) =>
			JSON.parse(
				new TextDecoder().decode(
					Uint8Array.from(
						pin(node, "var_ref", IPinType.Input).default_value ?? [],
					),
				),
			),
		);
		expect(refs).toEqual([variables[0].id, variables[0].id]);
	});

	test("a multi-instance sub-process runs its body under a loop node", () => {
		const { board } = process(`
			<startEvent id="s"/>
			<sequenceFlow id="f0" sourceRef="s" targetRef="sub"/>
			<subProcess id="sub" name="Each">
				<multiInstanceLoopCharacteristics isSequential="false"/>
				<startEvent id="ss"/>
				<sequenceFlow id="f1" sourceRef="ss" targetRef="t"/>
				<task id="t" name="Item"/>
				<sequenceFlow id="f2" sourceRef="t" targetRef="se"/>
				<endEvent id="se"/>
			</subProcess>`);
		assertFragmentInvariants(board);
		const loop = nodesNamed(board, "control_par_for_each")[0];
		expect(loop).toBeDefined();
		const body = execTarget(board, pin(loop, "exec_out", IPinType.Output));
		expect(body?.layer).toBe(layerNamed(board, "Item")?.id);
		const start = nodesNamed(board, "events_simple")[0];
		expect(execTarget(board, pin(start, "exec_out", IPinType.Output))?.id).toBe(
			loop.id,
		);
	});

	test("an error boundary on an HTTP service task is wired to the fetch error output", () => {
		const result = process(`
			<startEvent id="s"/>
			<sequenceFlow id="f0" sourceRef="s" targetRef="call"/>
			<serviceTask id="call" name="Call">
				<extensionElements><zeebe:taskDefinition xmlns:zeebe="http://camunda.org/schema/zeebe/1.0" type="io.camunda:http-json:1"/></extensionElements>
			</serviceTask>
			<boundaryEvent id="be" name="Failed" attachedToRef="call">
				<errorEventDefinition/>
			</boundaryEvent>
			<sequenceFlow id="f1" sourceRef="be" targetRef="e"/>
			<endEvent id="e"/>`);
		assertFragmentInvariants(result.board);
		const fetch = nodesNamed(result.board, "http_fetch")[0];
		expect(fetch).toBeDefined();
		const marker = execTarget(
			result.board,
			pin(fetch, "exec_error", IPinType.Output),
		);
		expect(marker?.comment).toContain("boundary event");
		expect(
			result.diagnostics.some((d) =>
				d.message.includes("wired to the error output"),
			),
		).toBe(true);
	});
});

/**
 * Real exports, dropped in as-is. Anything added to `bpmn/tests` is covered
 * from then on, which is the point: these are the files the importer is
 * actually judged by, and they use none of the executable-BPMN extensions the
 * hand-written fixtures do.
 */
describe("real-world diagrams", () => {
	const dir = resolve(import.meta.dir, "bpmn", "tests");
	const files = readdirSync(dir).filter((name) => name.endsWith(".bpmn"));
	const catalog = bpmnCatalogFixture();

	test("the directory holds files to check", () => {
		expect(files.length).toBeGreaterThan(0);
	});

	for (const file of files) {
		describe(file, () => {
			const defs = parseBpmn(readFileSync(resolve(dir, file), "utf-8"));
			const result = translateBpmn(defs, catalog);

			test("translates without an error diagnostic", () => {
				expect(
					result.diagnostics.filter((d) => d.level === "error"),
				).toHaveLength(0);
				expect(result.board.name).toBeTruthy();
			});

			test("produces a fragment the backend will accept", () => {
				assertFragmentInvariants(result.board);
			});

			test("accounts for every flow node in the file", () => {
				let expected = 0;
				for (const process of defs.processes) {
					expected += [...walkFlowNodes(process)].length;
				}
				expect(result.stats.totalNodes).toBe(expected);
				expect(result.stats.directMapped + result.stats.todo).toBeGreaterThan(
					0,
				);
			});

			test("gives every pool its own layer and every element a home", () => {
				const pools = defs.processes.filter((p) =>
					[...walkFlowNodes(p)].some(() => true),
				);
				if (pools.length > 1) {
					const roots = Object.values(result.board.layers).filter(
						(layer) => layer.parent_id === null,
					);
					expect(roots.length).toBeGreaterThanOrEqual(pools.length);
				}
				for (const node of Object.values(result.board.nodes)) {
					expect(
						node.layer === null ||
							(node.layer !== undefined && result.board.layers[node.layer]),
					).toBeTruthy();
				}
			});

			test("wires at least one connection per element that has a flow", () => {
				const flows = defs.processes.reduce(
					(sum, process) => sum + [...walkSequenceFlows(process)].length,
					0,
				);
				if (flows > 0) expect(result.stats.connections).toBeGreaterThan(0);
			});
		});
	}
});

describe("labelled exclusive gateways", () => {
	const result = translateBpmn(
		parseBpmn(
			readFileSync(
				resolve(import.meta.dir, "bpmn", "tests", "Car-Wash.bpmn"),
				"utf-8",
			),
		),
		bpmnCatalogFixture(),
	);
	const { board } = result;

	test("named branches with no condition become one Switch per gateway", () => {
		const switches = nodesNamed(board, "control_switch");
		expect(switches).toHaveLength(2);
		expect(nodesNamed(board, "control_branch")).toHaveLength(0);
	});

	test("the branch labels become the cases, and each case pin drives its branch", () => {
		const wash = nodesNamed(board, "control_switch").find((node) =>
			node.comment?.includes("Which wash program?"),
		);
		if (!wash) throw new Error("wash gateway missing");
		const cases = JSON.parse(
			new TextDecoder().decode(
				Uint8Array.from(pin(wash, "cases", IPinType.Input).default_value ?? []),
			),
		);
		expect(cases).toBe("Polish Plus,ECO");
		// The pin names must match what `control_switch::on_update` derives, or it
		// drops them — and the wire drawn here with them.
		const casePins = Object.values(wash.pins).filter((p) =>
			p.name.startsWith("case_"),
		);
		expect(casePins.map((p) => p.name).sort()).toEqual([
			"case_eco",
			"case_polish_plus",
		]);
		expect(casePins.map((p) => p.friendly_name).sort()).toEqual([
			"ECO",
			"Polish Plus",
		]);
		for (const casePin of casePins) {
			expect(casePin.connected_to).toHaveLength(1);
		}
	});
});
