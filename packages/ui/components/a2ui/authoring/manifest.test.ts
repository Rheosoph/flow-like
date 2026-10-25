import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import ts from "typescript";
import { COMPONENT_PROPS } from "../component-prop-manifest";
import {
	CHAT_UI_ACTIONS,
	CHAT_UI_COMPONENTS,
	COMPONENT_MANIFEST,
	componentSpec,
	propSpec,
} from "./manifest";

/**
 * Prop names where the Rust `*Props` structs (the manifest) and types.ts legitimately
 * differ. Anything else is drift: regenerate the manifest or fix the interface.
 */
const PROP_NAME_EXCEPTIONS: Record<
	string,
	{ readonly tsOnly?: readonly string[]; readonly rustOnly?: readonly string[] }
> = {
	// Reconciliation state the page runtime writes onto a live instance; never authored.
	widgetInstance: { tsOnly: ["runtimeChildUpdates"] },
	// The Rust struct keeps an unrendered styleOverride the TS interface never had.
	microWidgetInstance: { rustOnly: ["styleOverride"] },
};

const TYPES_FILE = fileURLToPath(new URL("../types.ts", import.meta.url));
const UNION_COMMENT = /^\/\/\s*("[^"]*"(?:\s*\|\s*"[^"]*")+)\s*$/;
const ALIAS_COMMENT = /^\/\/\s*([A-Z]\w*)\s*$/;

function stringLiteralUnion(node: ts.TypeNode): string[] | undefined {
	const members = ts.isUnionTypeNode(node) ? node.types : [node];
	const values: string[] = [];
	for (const member of members) {
		if (!ts.isLiteralTypeNode(member) || !ts.isStringLiteral(member.literal)) {
			return undefined;
		}
		values.push(member.literal.text);
	}
	return values.length > 1 ? values : undefined;
}

/** `component.prop` → the literal values types.ts documents (union type, alias or `// "a" | "b"`). */
function typesUnions(): Map<string, string[]> {
	const text = readFileSync(TYPES_FILE, "utf8");
	const source = ts.createSourceFile(TYPES_FILE, text, ts.ScriptTarget.Latest);
	const aliases = new Map<string, string[]>();
	const interfaces: ts.InterfaceDeclaration[] = [];
	source.forEachChild((node) => {
		if (ts.isTypeAliasDeclaration(node)) {
			const values = stringLiteralUnion(node.type);
			if (values) aliases.set(node.name.text, values);
		}
		if (ts.isInterfaceDeclaration(node)) interfaces.push(node);
	});
	const unions = new Map<string, string[]>();
	for (const declaration of interfaces) {
		const typeMember = declaration.members.find(
			(member) =>
				ts.isPropertySignature(member) &&
				member.name.getText(source) === "type" &&
				member.type !== undefined &&
				ts.isLiteralTypeNode(member.type),
		) as ts.PropertySignature | undefined;
		const literal = typeMember?.type as ts.LiteralTypeNode | undefined;
		if (!literal || !ts.isStringLiteral(literal.literal)) continue;
		const componentType = literal.literal.text;
		for (const member of declaration.members) {
			if (!ts.isPropertySignature(member) || !member.type) continue;
			const name = member.name.getText(source);
			if (name === "type") continue;
			const comments = (
				ts.getTrailingCommentRanges(text, member.end) ?? []
			).map((range) => text.slice(range.pos, range.end));
			const unionComment = comments
				.map((value) => UNION_COMMENT.exec(value)?.[1])
				.find((value) => value !== undefined);
			const aliasComment = comments
				.map((value) => ALIAS_COMMENT.exec(value)?.[1])
				.find((value) => value !== undefined);
			const values =
				stringLiteralUnion(member.type) ??
				(ts.isTypeReferenceNode(member.type)
					? aliases.get(member.type.typeName.getText(source))
					: undefined) ??
				(unionComment
					? [...unionComment.matchAll(/"([^"]*)"/g)].map(
							(match) => match[1] ?? "",
						)
					: undefined) ??
				(aliasComment ? aliases.get(aliasComment) : undefined);
			if (values) unions.set(`${componentType}.${name}`, values);
		}
	}
	return unions;
}

describe("component manifest", () => {
	test("indexes every component and its props", () => {
		expect(componentSpec("text")?.props.get("content")).toMatchObject({
			kind: "bound",
			required: true,
		});
		expect(propSpec("overlay", "baseComponentId")?.kind).toBe("plain");
		expect(propSpec("overlay", "overlays")?.kind).toBe("list");
		expect(propSpec("text", "notAProp")).toBeUndefined();
		expect(componentSpec("notAType")).toBeUndefined();
	});

	test("the chat policy lists known components and the two chat actions", () => {
		for (const type of CHAT_UI_COMPONENTS) {
			expect(componentSpec(type), type).toBeDefined();
		}
		for (const blocked of ["iframe", "widgetInstance", "modal", "fileInput"]) {
			expect(CHAT_UI_COMPONENTS.has(blocked)).toBe(false);
		}
		expect([...CHAT_UI_ACTIONS].sort()).toEqual([
			"external_link",
			"send_message",
		]);
	});
});

describe("manifest ↔ types.ts drift", () => {
	test("both sides know the same component types", () => {
		expect(
			COMPONENT_MANIFEST.components.map((spec) => spec.type).sort(),
		).toEqual(Object.keys(COMPONENT_PROPS).sort());
	});

	test("prop names match per type, apart from the listed exceptions", () => {
		const props = COMPONENT_PROPS as Record<string, readonly string[]>;
		const drift: string[] = [];
		for (const spec of COMPONENT_MANIFEST.components) {
			const rust = new Set(spec.props.map((prop) => prop.name));
			const typescript = new Set(props[spec.type] ?? []);
			const exceptions = PROP_NAME_EXCEPTIONS[spec.type] ?? {};
			const tsOnly = [...typescript].filter((name) => !rust.has(name)).sort();
			const rustOnly = [...rust].filter((name) => !typescript.has(name)).sort();
			const expectedTsOnly = [...(exceptions.tsOnly ?? [])].sort();
			const expectedRustOnly = [...(exceptions.rustOnly ?? [])].sort();
			if (tsOnly.join() !== expectedTsOnly.join()) {
				drift.push(`${spec.type}: only in types.ts: ${tsOnly.join(", ")}`);
			}
			if (rustOnly.join() !== expectedRustOnly.join()) {
				drift.push(
					`${spec.type}: only in the manifest: ${rustOnly.join(", ")}`,
				);
			}
		}
		expect(drift).toEqual([]);
	});

	test('manifest `values` equal the literal unions in types.ts (incl. `// "a" | "b"`)', () => {
		const unions = typesUnions();
		const mismatches: string[] = [];
		const undocumented: string[] = [];
		let compared = 0;
		for (const spec of COMPONENT_MANIFEST.components) {
			for (const prop of spec.props) {
				if (!prop.values?.length) continue;
				const key = `${spec.type}.${prop.name}`;
				const union = unions.get(key);
				if (!union) {
					undocumented.push(key);
					continue;
				}
				compared++;
				const sameValues =
					[...union].sort().join("|") === [...prop.values].sort().join("|");
				if (!sameValues) {
					mismatches.push(
						`${key}: types.ts ${union.join("|")} ≠ manifest ${prop.values.join("|")}`,
					);
				}
			}
		}
		expect(mismatches).toEqual([]);
		expect(undocumented).toEqual([]);
		expect(compared).toBeGreaterThan(50);
	});
});
