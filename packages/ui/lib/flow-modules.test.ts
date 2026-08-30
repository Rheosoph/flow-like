import { describe, expect, it } from "bun:test";
import {
	FLOWSCRIPT_KEYWORDS,
	MAIN_FILE_ID,
	MAIN_FILE_LABEL,
	activeModuleId,
	boardFlowScriptScope,
	boardModules,
	fileModuleId,
	moduleFileId,
	modulePathSegments,
	siblingModules,
	toModuleIdent,
	validateModuleName,
} from "./flow-modules";
import { type ILayer, ILayerType } from "./schema/flow/board";

function layer(
	id: string,
	parentId?: string | null,
	type: ILayerType = ILayerType.Module,
	name?: string,
): ILayer {
	return {
		id,
		parent_id: parentId ?? null,
		name: name ?? id,
		type,
		nodes: {},
		variables: {},
		comments: {},
		coordinates: [0, 0, 0],
		pins: {},
	} as unknown as ILayer;
}

function layerMap(...entries: ILayer[]): Record<string, ILayer> {
	return Object.fromEntries(entries.map((entry) => [entry.id, entry]));
}

describe("toModuleIdent", () => {
	it("camelCases separators away", () => {
		expect(toModuleIdent("checkout")).toBe("checkout");
		expect(toModuleIdent("Payment Provider")).toBe("paymentProvider");
		expect(toModuleIdent("payment_provider")).toBe("paymentProvider");
		expect(toModuleIdent("payment-provider")).toBe("paymentProvider");
		expect(toModuleIdent("Payment::Provider")).toBe("paymentProvider");
	});

	it("lowercases only the first character", () => {
		expect(toModuleIdent("HTTPClient")).toBe("hTTPClient");
	});

	it("prefixes a digit-leading name, which would lex as a number", () => {
		expect(toModuleIdent("3ds secure")).toBe("_3dsSecure");
	});

	it("returns an empty ident for a name with nothing to keep", () => {
		expect(toModuleIdent("  ")).toBe("");
		expect(toModuleIdent("///")).toBe("");
	});
});

describe("modulePathSegments", () => {
	const layers = layerMap(
		layer("checkout", null, ILayerType.Module, "Checkout"),
		layer("payments", "checkout", ILayerType.Module, "Payments"),
		layer("fn", "payments", ILayerType.Function),
		layer("collapsed", null, ILayerType.Collapsed),
		layer("under_collapsed", "collapsed", ILayerType.Module, "Nested"),
	);

	it("walks module ancestors outermost first", () => {
		expect(modulePathSegments(layers, "payments")).toEqual([
			"checkout",
			"payments",
		]);
	});

	it("is empty for a layer that is not a module", () => {
		expect(modulePathSegments(layers, "fn")).toEqual([]);
		expect(modulePathSegments(layers, "missing")).toEqual([]);
	});

	it("stops at the first non-module ancestor", () => {
		expect(modulePathSegments(layers, "under_collapsed")).toEqual(["nested"]);
	});

	it("terminates on a cyclic parent chain", () => {
		const cyclic = layerMap(layer("a", "b"), layer("b", "a"));
		expect(modulePathSegments(cyclic, "a")).toEqual(["b", "a"]);
	});
});

describe("boardModules", () => {
	it("lists every module with its file label, sorted by path", () => {
		const layers = layerMap(
			layer("payments", "checkout", ILayerType.Module, "Payments"),
			layer("checkout", null, ILayerType.Module, "Checkout"),
			layer("audit", null, ILayerType.Module, "Audit"),
			layer("fn", null, ILayerType.Function),
			layer("collapsed", null, ILayerType.Collapsed),
		);

		expect(boardModules(layers)).toEqual([
			{ id: "audit", name: "Audit", pathLabel: "audit.flow" },
			{ id: "checkout", name: "Checkout", pathLabel: "checkout.flow" },
			{
				id: "payments",
				name: "Payments",
				pathLabel: "checkout/payments.flow",
			},
		]);
	});

	it("breaks a label tie by id, so the order never flickers", () => {
		const layers = layerMap(
			layer("b", null, ILayerType.Module, "Same"),
			layer("a", null, ILayerType.Module, "same"),
		);
		expect(boardModules(layers).map((module) => module.id)).toEqual(["a", "b"]);
	});

	it("is empty for a board without modules", () => {
		expect(boardModules(undefined)).toEqual([]);
		expect(
			boardModules(layerMap(layer("fn", null, ILayerType.Function))),
		).toEqual([]);
	});
});

describe("activeModuleId", () => {
	const layers = layerMap(
		layer("checkout", null, ILayerType.Module, "Checkout"),
		layer("payments", "checkout", ILayerType.Module, "Payments"),
		layer("fn", "checkout", ILayerType.Function),
		layer("collapsed", "fn", ILayerType.Collapsed),
		layer("global_fn", null, ILayerType.Function),
	);

	it("is null at the board root", () => {
		expect(activeModuleId(undefined, undefined, layers)).toBeNull();
	});

	it("is the module itself when a module is open", () => {
		expect(activeModuleId("checkout", "checkout", layers)).toBe("checkout");
		expect(activeModuleId("checkout/payments", "payments", layers)).toBe(
			"payments",
		);
	});

	it("is the owning module of a module-local function", () => {
		expect(activeModuleId("fn", "fn", layers)).toBe("checkout");
	});

	it("walks past intermediate layers to the owning module", () => {
		expect(activeModuleId("fn/collapsed", "collapsed", layers)).toBe(
			"checkout",
		);
	});

	it("is null inside a board-global function", () => {
		expect(activeModuleId("global_fn", "global_fn", layers)).toBeNull();
	});

	it("falls back to the last path segment when currentLayer is missing", () => {
		expect(activeModuleId("checkout/payments", undefined, layers)).toBe(
			"payments",
		);
	});
});

describe("siblingModules", () => {
	const layers = layerMap(
		layer("checkout", null),
		layer("audit", null),
		layer("payments", "checkout"),
		layer("fn", null, ILayerType.Function),
	);

	it("lists top-level modules for a null parent", () => {
		expect(
			siblingModules(layers, null)
				.map((layer) => layer.id)
				.toSorted(),
		).toEqual(["audit", "checkout"]);
	});

	it("lists nested modules for a module parent", () => {
		expect(siblingModules(layers, "checkout").map((layer) => layer.id)).toEqual(
			["payments"],
		);
	});
});

describe("validateModuleName", () => {
	const layers = layerMap(
		layer("checkout", null, ILayerType.Module, "Checkout"),
		layer("payments", "checkout", ILayerType.Module, "Payments"),
	);

	it("accepts a fresh name", () => {
		expect(validateModuleName("Billing", layers, null)).toBeNull();
		expect(validateModuleName("Payments", layers, null)).toBeNull();
	});

	it("rejects an empty name", () => {
		expect(validateModuleName("   ", layers, null)).toBe("empty");
	});

	it("rejects a name that camelCases to nothing usable", () => {
		expect(validateModuleName("///", layers, null)).toBe("invalid_identifier");
	});

	it("rejects FlowScript keywords", () => {
		for (const keyword of FLOWSCRIPT_KEYWORDS) {
			expect(validateModuleName(keyword, layers, null)).toBe("reserved");
		}
		expect(validateModuleName("Function", layers, null)).toBe("reserved");
	});

	it("rejects a caller-supplied reserved root", () => {
		expect(validateModuleName("string", layers, null, ["string"])).toBe(
			"reserved",
		);
	});

	it("rejects a sibling collision, case- and separator-insensitively", () => {
		expect(validateModuleName("checkout", layers, null)).toBe("duplicate");
		expect(validateModuleName("Check Out", layers, null)).toBe("duplicate");
		expect(validateModuleName("Payments", layers, "checkout")).toBe(
			"duplicate",
		);
	});

	it("allows the same name under a different parent", () => {
		expect(validateModuleName("Checkout", layers, "checkout")).toBeNull();
	});

	it("lets a rename keep its own name", () => {
		expect(
			validateModuleName("Checkout", layers, null, undefined, "checkout"),
		).toBeNull();
	});
});

describe("MAIN_FILE_LABEL", () => {
	it("is the untranslated root file name", () => {
		expect(MAIN_FILE_LABEL).toBe("main.flow");
	});
});

describe("file ids", () => {
	it("maps the root to `main` and back", () => {
		expect(moduleFileId(null)).toBe(MAIN_FILE_ID);
		expect(moduleFileId(undefined)).toBe(MAIN_FILE_ID);
		expect(moduleFileId("checkout")).toBe("checkout");
		expect(fileModuleId(MAIN_FILE_ID)).toBeUndefined();
		expect(fileModuleId(undefined)).toBeUndefined();
		expect(fileModuleId("checkout")).toBe("checkout");
	});
});

describe("boardFlowScriptScope", () => {
	const layers = layerMap(
		layer("checkout", null, ILayerType.Module, "Checkout"),
		layer("payments", "checkout", ILayerType.Module, "Payments"),
		layer("total", "checkout", ILayerType.Function, "Order Total"),
		layer("capture", "payments", ILayerType.Function, "Capture"),
		layer("helper", null, ILayerType.Function, "Root Helper"),
		layer("collapsed", "payments", ILayerType.Collapsed, "Grouped"),
		layer("nested", "collapsed", ILayerType.Function, "Deep Fn"),
	);

	it("names every module by its namespace path", () => {
		expect(boardFlowScriptScope(layers).modules.toSorted()).toEqual([
			"checkout",
			"checkout::payments",
		]);
	});

	it("files each function under its owning module, root functions under ''", () => {
		const { functionsByModule } = boardFlowScriptScope(layers);
		expect(functionsByModule[""]).toEqual(["rootHelper"]);
		expect(functionsByModule.checkout).toEqual(["orderTotal"]);
		// A function inside a plain layer still belongs to the module around it.
		expect(functionsByModule["checkout::payments"]?.toSorted()).toEqual([
			"capture",
			"deepFn",
		]);
	});

	it("is empty for a board without modules", () => {
		expect(boardFlowScriptScope(undefined)).toEqual({
			modules: [],
			functionsByModule: {},
		});
	});
});
