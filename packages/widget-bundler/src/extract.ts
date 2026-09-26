import { existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import type {
	WidgetCapabilities,
	WidgetCspDirective,
	WidgetCspPurpose,
	WidgetNetworkInput,
	WidgetUrlTemplate,
} from "@flow-like/widget-sdk";
import { isLlmKind } from "@flow-like/widget-sdk/llm";
import { validateInputValue } from "@flow-like/widget-sdk/validate";
import {
	type CompletedConfig,
	DEFAULT_CONFIG,
	SchemaGenerator,
	createFormatter,
	createParser,
	createProgram,
} from "ts-json-schema-generator";
import ts from "typescript";
import {
	BASE_CONTRACT_VERSION,
	CONTRACT_VERSION,
	type ContractEvent,
	type ContractInput,
	type ContractQuery,
	type JsonValue,
	type WidgetContract,
	normalizeCspPurposes,
	validateContract,
	validatePublishedCsp,
} from "./contract-types";
import {
	cspReasonProblem,
	foldWidgetCspReason,
	reasonContainsAddress,
	validateWidgetCspReason,
} from "./csp-reason";
import {
	CSP_DIRECTIVES,
	CSP_PURPOSE_KEYS,
	CSP_SOURCE_REJECTION_MESSAGES,
	type WidgetInputPathSegment,
	flattenCspPurposes,
	isCspDirective,
	normalizeCspSource,
	parseWidgetInputPath,
	validateCspSource,
} from "./csp-source";
import { WILDCARD_PUBLIC_SUFFIX_MESSAGE, validateWildcardBases } from "./psl";

type JsonObject = { [key: string]: JsonValue };

export interface WidgetSizingConfig {
	defaultHeight?: number;
	resizable?: boolean;
	maxHeight?: number;
}

export interface ExtractedWidgetConfig {
	id: string;
	name: string;
	description: string;
	sizing?: WidgetSizingConfig;
	capabilities?: WidgetCapabilities;
	/** Normalized, canonical purpose groups; absent when none are declared */
	csp?: WidgetCspPurpose[];
	fixtures?: Record<string, JsonValue>;
}

export interface ExtractResult {
	contract: WidgetContract;
	config: ExtractedWidgetConfig;
	warnings: string[];
}

interface SectionMember {
	name: string;
	node: ts.PropertySignature;
	typeNode: ts.TypeNode;
	optional: boolean;
}

interface ResolvedSection {
	name: string;
	declaration: ts.InterfaceDeclaration | ts.TypeAliasDeclaration;
}

/**
 * Statically derive a widget's `contract.json` and evaluated config from its
 * `widget.config.ts` (`export default defineWidget<Inputs, Events, Queries>({...})`).
 */
export function extractContract(widgetConfigPath: string): ExtractResult {
	const absPath = resolve(widgetConfigPath);
	if (!existsSync(absPath)) {
		throw new Error(`Widget config not found: ${absPath}`);
	}

	// Package exports, path aliases and ESM resolution must match the widget build.
	const tsconfig = ts.findConfigFile(dirname(absPath), ts.sys.fileExists);
	const generatorConfig: CompletedConfig = {
		...DEFAULT_CONFIG,
		path: absPath,
		...(tsconfig && { tsconfig }),
		skipTypeCheck: true,
		jsDoc: "extended",
		extraTags: ["geometry", "llm", "mutation"],
		topRef: false,
		expose: "all",
		additionalProperties: true,
		sortProps: true,
	};
	const program = createProgram(generatorConfig);
	const checker = program.getTypeChecker();
	const sourceFile = program
		.getSourceFiles()
		.find((sf) => resolve(sf.fileName) === absPath);
	if (!sourceFile) {
		throw new Error(`Failed to load ${absPath} into the TypeScript program`);
	}

	const call = findDefineWidgetCall(sourceFile, absPath);
	const [inputsArg, eventsArg, queriesArg] = call.typeArguments ?? [];
	const inputsSection = inputsArg
		? resolveSectionType(inputsArg, checker, "Inputs", absPath)
		: null;
	const eventsSection = eventsArg
		? resolveSectionType(eventsArg, checker, "Events", absPath)
		: null;
	const queriesSection = queriesArg
		? resolveSectionType(queriesArg, checker, "Queries", absPath)
		: null;

	const configLiteral = unwrapExpression(
		call.arguments[0] ?? missingConfig(absPath),
	);
	if (!ts.isObjectLiteralExpression(configLiteral)) {
		throw new Error(
			`defineWidget(...) in ${absPath} must be called with an object literal`,
		);
	}
	const config = readWidgetConfig(
		evaluateObjectLiteral(configLiteral, ""),
		absPath,
	);

	const generator = new SchemaGenerator(
		program,
		createParser(program, generatorConfig),
		createFormatter(generatorConfig),
		generatorConfig,
	);
	const schemaFor = (typeName: string): JsonObject => {
		try {
			return generator.createSchema(typeName) as unknown as JsonObject;
		} catch (e) {
			throw new Error(
				`Failed to derive a JSON Schema for type '${typeName}' in ${absPath}: ${e instanceof Error ? e.message : e}`,
			);
		}
	};

	const warnings: string[] = [];
	const inputs = inputsSection
		? extractInputs(inputsSection, schemaFor, checker, config.id, warnings)
		: {};
	const events = eventsSection
		? extractEvents(eventsSection, schemaFor, checker)
		: {};
	const queries = queriesSection
		? extractQueries(queriesSection, schemaFor, checker)
		: {};

	const contract: WidgetContract = {
		contractVersion: config.csp ? CONTRACT_VERSION : BASE_CONTRACT_VERSION,
		id: config.id,
		...(config.capabilities && { capabilities: config.capabilities }),
		...(config.csp && { csp: config.csp }),
		inputs,
		events,
		queries,
		sizing: {
			defaultHeight: config.sizing?.defaultHeight ?? 320,
			resizable: config.sizing?.resizable ?? true,
			...(config.sizing?.maxHeight !== undefined && {
				maxHeight: config.sizing.maxHeight,
			}),
		},
	};

	const errors = validateContract(contract);
	if (errors.length === 0) {
		errors.push(
			...validatePublishedCsp(contract),
			...networkInputSchemaErrors(contract),
		);
	}
	if (errors.length > 0) {
		throw new Error(
			`Invalid contract for widget '${config.id}' (${absPath}): ${errors.join("; ")}`,
		);
	}
	warnings.push(...networkInputWarnings(contract));

	return { contract, config, warnings };
}

function missingConfig(path: string): never {
	throw new Error(
		`defineWidget(...) in ${path} is missing its config argument`,
	);
}

function unwrapExpression(expr: ts.Expression): ts.Expression {
	let current = expr;
	while (
		ts.isAsExpression(current) ||
		ts.isSatisfiesExpression(current) ||
		ts.isParenthesizedExpression(current) ||
		ts.isNonNullExpression(current)
	) {
		current = current.expression;
	}
	return current;
}

function findDefineWidgetCall(
	sourceFile: ts.SourceFile,
	path: string,
): ts.CallExpression {
	for (const statement of sourceFile.statements) {
		if (!ts.isExportAssignment(statement) || statement.isExportEquals) continue;
		const expr = unwrapExpression(statement.expression);
		if (!ts.isCallExpression(expr)) continue;
		const callee = unwrapExpression(expr.expression);
		const calleeName = ts.isIdentifier(callee)
			? callee.text
			: ts.isPropertyAccessExpression(callee)
				? callee.name.text
				: null;
		if (calleeName === "defineWidget") return expr;
	}
	throw new Error(
		`${path} must contain \`export default defineWidget<Inputs, Events, Queries>({ ... })\``,
	);
}

function resolveSectionType(
	typeArg: ts.TypeNode,
	checker: ts.TypeChecker,
	label: string,
	path: string,
): ResolvedSection | null {
	if (ts.isTypeLiteralNode(typeArg)) {
		if (typeArg.members.length === 0) return null;
		throw new Error(
			`${label} type argument in ${path} is an inline type literal; declare a named interface or type alias instead`,
		);
	}
	if (!ts.isTypeReferenceNode(typeArg)) {
		throw new Error(
			`${label} type argument in ${path} must be a named interface or type alias`,
		);
	}
	const declaration = resolveTypeDeclaration(typeArg.typeName, checker);
	if (!declaration) {
		throw new Error(
			`Cannot resolve ${label} type '${typeArg.typeName.getText()}' in ${path}; it must be an interface or type alias declared in this file or imported from a sibling file`,
		);
	}
	return { name: declaration.name.text, declaration };
}

function resolveTypeDeclaration(
	typeName: ts.EntityName,
	checker: ts.TypeChecker,
): ts.InterfaceDeclaration | ts.TypeAliasDeclaration | null {
	let symbol = checker.getSymbolAtLocation(typeName);
	if (!symbol) return null;
	if (symbol.flags & ts.SymbolFlags.Alias) {
		symbol = checker.getAliasedSymbol(symbol);
	}
	for (const declaration of symbol.declarations ?? []) {
		if (
			ts.isInterfaceDeclaration(declaration) ||
			ts.isTypeAliasDeclaration(declaration)
		) {
			return declaration;
		}
	}
	return null;
}

function sectionMembers(
	section: ResolvedSection,
	label: string,
): SectionMember[] {
	let members: ts.NodeArray<ts.TypeElement>;
	if (ts.isInterfaceDeclaration(section.declaration)) {
		members = section.declaration.members;
	} else {
		const aliased = section.declaration.type;
		if (!ts.isTypeLiteralNode(aliased)) {
			throw new Error(
				`${label} type '${section.name}' must be an interface or an object type literal alias`,
			);
		}
		members = aliased.members;
	}

	const result: SectionMember[] = [];
	for (const member of members) {
		if (!ts.isPropertySignature(member)) {
			throw new Error(
				`${label} type '${section.name}' may only contain plain properties (found ${ts.SyntaxKind[member.kind]})`,
			);
		}
		const name =
			ts.isIdentifier(member.name) || ts.isStringLiteral(member.name)
				? member.name.text
				: null;
		if (name === null) {
			throw new Error(
				`${label} type '${section.name}' contains a computed property name; only plain identifiers are supported`,
			);
		}
		if (!member.type) {
			throw new Error(
				`Property '${name}' of ${label} type '${section.name}' must have an explicit type annotation`,
			);
		}
		result.push({
			name,
			node: member,
			typeNode: unwrapTypeNode(member.type),
			optional: member.questionToken !== undefined,
		});
	}
	return result;
}

function unwrapTypeNode(node: ts.TypeNode): ts.TypeNode {
	let current = node;
	while (ts.isParenthesizedTypeNode(current)) {
		current = current.type;
	}
	return current;
}

function isVoidLike(node: ts.TypeNode): boolean {
	return (
		node.kind === ts.SyntaxKind.VoidKeyword ||
		node.kind === ts.SyntaxKind.UndefinedKeyword ||
		node.kind === ts.SyntaxKind.NeverKeyword
	);
}

function memberDescription(member: ts.PropertySignature): string | undefined {
	for (const doc of ts.getJSDocCommentsAndTags(member)) {
		if (!ts.isJSDoc(doc)) continue;
		const text = ts.getTextOfJSDocComment(doc.comment)?.trim();
		if (text) return text;
	}
	return undefined;
}

function memberHasTag(member: ts.PropertySignature, tagName: string): boolean {
	return ts.getJSDocTags(member).some((tag) => tag.tagName.text === tagName);
}

function isJsonObject(value: JsonValue | undefined): value is JsonObject {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function requireSchemaObject(value: JsonValue, context: string): JsonObject {
	if (!isJsonObject(value)) {
		throw new Error(`Generated schema for ${context} is not an object`);
	}
	return value;
}

function schemaDefinitions(schema: JsonObject): Record<string, JsonValue> {
	return isJsonObject(schema.definitions) ? schema.definitions : {};
}

function schemaProperties(schema: JsonObject): Record<string, JsonValue> {
	return isJsonObject(schema.properties) ? schema.properties : {};
}

function schemaRequired(schema: JsonObject): Set<string> {
	const required = schema.required;
	return new Set(
		Array.isArray(required)
			? required.filter((r): r is string => typeof r === "string")
			: [],
	);
}

const GEOMETRY_KINDS = new Set([
	"Point",
	"LineString",
	"Polygon",
	"MultiPoint",
	"MultiLineString",
	"MultiPolygon",
	"GeometryCollection",
]);

/** Geometry annotations select the shared geometry validator, including collections. */
function geometrySchema(
	schema: JsonObject,
	definitions: Record<string, JsonValue>,
	context: string,
): JsonObject {
	const kind = schema.geometry;
	if (
		typeof kind !== "string" ||
		(kind !== "Any" && !GEOMETRY_KINDS.has(kind))
	) {
		throw new Error(
			`Invalid @geometry '${String(kind)}' for ${context}; expected Any or ${[...GEOMETRY_KINDS].join(", ")}`,
		);
	}

	const objectShape = (
		value: JsonObject,
		visited = new Set<string>(),
		expectedKind = kind,
	): JsonObject => {
		let shape = value;
		while (typeof shape.$ref === "string" && !visited.has(shape.$ref)) {
			visited.add(shape.$ref);
			const definition = resolveDefinition(shape.$ref, definitions, context);
			if (!isJsonObject(definition)) break;
			const { $ref: _ref, ...rest } = shape;
			shape = { ...definition, ...rest };
		}
		if (typeof shape.$ref === "string" && visited.has(shape.$ref)) return shape;
		if (shape.type !== undefined && shape.type !== "object") {
			throw new Error(
				`@geometry ${kind} for ${context} must annotate a geometry object type; annotate the element type for arrays and maps`,
			);
		}
		let hasBranches = false;
		for (const key of ["anyOf", "oneOf", "allOf"]) {
			const branches = shape[key];
			if (Array.isArray(branches) && branches.length > 0) {
				hasBranches = true;
				for (const branch of branches) {
					if (isJsonObject(branch))
						objectShape(branch, new Set(visited), expectedKind);
				}
			}
		}
		if (!hasBranches) {
			const properties = schemaProperties(shape);
			const type = properties.type;
			const declaredKind = isJsonObject(type)
				? (type.const ??
					(Array.isArray(type.enum) && type.enum.length === 1
						? type.enum[0]
						: undefined))
				: undefined;
			if (
				typeof declaredKind !== "string" ||
				!GEOMETRY_KINDS.has(declaredKind) ||
				(expectedKind !== "Any" && declaredKind !== expectedKind) ||
				properties[
					declaredKind === "GeometryCollection" ? "geometries" : "coordinates"
				] === undefined
			) {
				throw new Error(
					`@geometry ${kind} for ${context} must describe a compatible GeoJSON geometry object; check its type discriminant and coordinates or geometries, and annotate the element type for maps`,
				);
			}
			const checkCoordinates = (
				value: JsonValue | undefined,
				depth: number,
			): void => {
				let field = value;
				const refs = new Set<string>();
				while (
					isJsonObject(field) &&
					typeof field.$ref === "string" &&
					!refs.has(field.$ref)
				) {
					refs.add(field.$ref);
					field = resolveDefinition(field.$ref, definitions, context);
				}
				const expected = depth > 0 ? "array" : "number";
				if (
					!isJsonObject(field) ||
					(field.type !== expected &&
						!(depth === 0 && field.type === "integer"))
				) {
					throw new Error(
						`@geometry ${kind} for ${context} has incompatible coordinates or geometries; expected ${expected}`,
					);
				}
				if (depth === 0) return;
				const items = Array.isArray(field.items) ? field.items : [field.items];
				if (items.length === 0)
					throw new Error(
						`@geometry ${kind} for ${context} requires an element type`,
					);
				for (const item of items) {
					if (declaredKind === "GeometryCollection") {
						if (!isJsonObject(item))
							throw new Error(
								`@geometry ${kind} for ${context} requires geometry elements`,
							);
						objectShape(item, new Set(visited), "Any");
					} else {
						checkCoordinates(item, depth - 1);
					}
				}
			};
			const dimensions: Record<string, number> = {
				Point: 1,
				LineString: 2,
				Polygon: 3,
				MultiPoint: 2,
				MultiLineString: 3,
				MultiPolygon: 4,
				GeometryCollection: 1,
			};
			checkCoordinates(
				properties[
					declaredKind === "GeometryCollection" ? "geometries" : "coordinates"
				],
				dimensions[declaredKind] ?? 1,
			);
		}
		return shape;
	};
	const shape = objectShape(schema);

	// The contract carries a schema extension; the host converts it into the
	// compact flow:geometry pin marker. The SDK validates the geometry profile.
	const result: JsonObject = {
		type: "object",
		"x-flow-like-type": "geometry",
	};
	if (kind !== "Any") result["x-geometry"] = kind;
	for (const key of [
		"title",
		"description",
		"default",
		"examples",
		"deprecated",
	]) {
		if (shape[key] !== undefined) result[key] = shape[key];
	}
	return result;
}

/**
 * `@llm History`, `Response` or `ResponseChunk` selects a native Flow-Like
 * model type. The contract keeps only the marker: Query Widget gives the pin
 * the native schema, so model node outputs connect, and the SDK validates
 * values against that schema.
 */
function llmSchema(schema: JsonObject, context: string): JsonObject {
	const kind = schema.llm;
	if (!isLlmKind(kind)) {
		throw new Error(
			`Invalid @llm '${String(kind)}' for ${context}; expected History, Response or ResponseChunk`,
		);
	}
	if (schema.type !== undefined && schema.type !== "object") {
		throw new Error(
			`@llm ${kind} for ${context} must annotate an object type; annotate the element type for arrays and maps`,
		);
	}
	const result: JsonObject = {
		type: "object",
		"x-flow-like-type": "llm",
		"x-llm": kind,
	};
	for (const key of [
		"title",
		"description",
		"default",
		"examples",
		"deprecated",
	]) {
		if (schema[key] !== undefined) result[key] = schema[key];
	}
	return result;
}

function resolveDefinition(
	ref: string,
	definitions: Record<string, JsonValue>,
	context: string,
): JsonValue {
	const prefix = "#/definitions/";
	if (!ref.startsWith(prefix)) {
		throw new Error(
			`Unsupported $ref '${ref}' while inlining the schema for ${context}`,
		);
	}
	const encoded = ref.slice(prefix.length);
	const definition =
		definitions[decodeURIComponent(encoded)] ?? definitions[encoded];
	if (definition === undefined) {
		throw new Error(
			`Unresolvable $ref '${ref}' while inlining the schema for ${context}`,
		);
	}
	return definition;
}

/**
 * Recursively resolve `#/definitions/...` refs so every emitted schema is
 * standalone (the runtime validator does not support `$ref`). Fails on
 * recursive types.
 */
function inlineRefs(
	value: JsonValue,
	definitions: Record<string, JsonValue>,
	stack: string[],
	context: string,
	schemaMap = false,
): JsonValue {
	if (Array.isArray(value)) {
		return value.map((item) => inlineRefs(item, definitions, stack, context));
	}
	if (!isJsonObject(value)) return value;
	if (schemaMap) {
		return Object.fromEntries(
			Object.entries(value).map(([key, entry]) => [
				key,
				inlineRefs(entry, definitions, stack, context),
			]),
		);
	}
	if (value.geometry !== undefined) {
		return geometrySchema(value, definitions, context);
	}
	if (value.llm !== undefined) {
		return llmSchema(value, context);
	}

	const { $ref, $schema, definitions: _nested, ...rest } = value;
	void $schema;
	void _nested;

	const inlinedRest: JsonObject = {};
	for (const [key, entry] of Object.entries(rest)) {
		// Annotation values are data, so a default containing "geometry" or
		// "$ref" must not be interpreted as a schema.
		inlinedRest[key] = ["default", "examples", "enum", "const"].includes(key)
			? entry
			: inlineRefs(
					entry,
					definitions,
					stack,
					context,
					["properties", "patternProperties", "dependentSchemas"].includes(key),
				);
	}

	if (typeof $ref !== "string") return inlinedRest;

	const definition = resolveDefinition($ref, definitions, context);
	const encoded = $ref.slice("#/definitions/".length);
	const key = decodeURIComponent(encoded);
	if (stack.includes(key)) {
		throw new Error(
			`Recursive type detected while inlining the schema for ${context} (cycle: ${[...stack, key].join(" -> ")}); widget contract schemas must be non-recursive`,
		);
	}
	const inlinedDef = inlineRefs(
		definition,
		definitions,
		[...stack, key],
		context,
	);
	if (!isJsonObject(inlinedDef)) return inlinedDef;
	return { ...inlinedDef, ...inlinedRest };
}

function extractInputs(
	section: ResolvedSection,
	schemaFor: (name: string) => JsonObject,
	checker: ts.TypeChecker,
	widgetId: string,
	warnings: string[],
): Record<string, ContractInput> {
	const members = sectionMembers(section, "Inputs");
	if (members.length === 0) return {};
	const schema = schemaFor(section.name);
	const properties = schemaProperties(schema);
	const required = schemaRequired(schema);
	const definitions = schemaDefinitions(schema);

	const inputs: Record<string, ContractInput> = {};
	for (const member of members) {
		if (isVoidLike(member.typeNode)) {
			throw new Error(
				`Input '${member.name}' of widget '${widgetId}' cannot be void/undefined/never`,
			);
		}
		const propertySchema = properties[member.name];
		if (propertySchema === undefined) {
			throw new Error(
				`No schema was generated for input '${member.name}' of widget '${widgetId}'`,
			);
		}
		const inlined = inlineRefs(
			propertySchema,
			definitions,
			[],
			`input '${member.name}' of widget '${widgetId}'`,
		);
		const optional = member.optional || !required.has(member.name);
		const input = mapInputSchema(
			isJsonObject(inlined) ? inlined : {},
			optional,
		);
		if (input.default !== undefined && containsGeometrySchema(input.schema)) {
			const result = validateInputValue(input, input.default);
			if (!result.valid) {
				throw new Error(
					`Invalid @default for geometry input '${member.name}' of widget '${widgetId}': ${result.errors.join("; ")}`,
				);
			}
		}
		if (!optional && input.default === undefined) {
			warnings.push(
				`Input '${member.name}' of widget '${widgetId}' has no @default and is not optional; standalone dev and generated pin defaults will have no value`,
			);
		}
		inputs[member.name] = input;
	}
	return inputs;
}

function containsGeometrySchema(value: JsonValue | undefined): boolean {
	if (Array.isArray(value)) return value.some(containsGeometrySchema);
	if (!isJsonObject(value)) return false;
	if (value["x-flow-like-type"] === "geometry") return true;
	return Object.entries(value).some(
		([key, entry]) =>
			!["default", "examples", "enum", "const"].includes(key) &&
			containsGeometrySchema(entry),
	);
}

function mapInputSchema(schema: JsonObject, optional: boolean): ContractInput {
	const type = schema.type;
	const enumValues = Array.isArray(schema.enum) ? schema.enum : null;
	const constValue = schema.const;

	let input: ContractInput;
	if (
		type === "string" &&
		enumValues &&
		enumValues.length > 0 &&
		enumValues.every((v): v is string => typeof v === "string")
	) {
		input = { type: "enum", choices: enumValues };
	} else if (type === "string" && typeof constValue === "string") {
		input = { type: "enum", choices: [constValue] };
	} else if (type === "string" && !enumValues) {
		input = { type: "string" };
	} else if (type === "boolean" && !enumValues && constValue === undefined) {
		input = { type: "boolean" };
	} else if (type === "integer" && !enumValues && constValue === undefined) {
		input = { type: "integer" };
	} else if (type === "number" && !enumValues && constValue === undefined) {
		input = { type: "number" };
	} else {
		input = { type: "json", schema };
	}

	if (typeof schema.description === "string") {
		input.description = schema.description;
	}
	if (schema.default !== undefined) {
		input.default = schema.default;
	}
	if (input.type === "number" || input.type === "integer") {
		if (typeof schema.minimum === "number") input.min = schema.minimum;
		if (typeof schema.maximum === "number") input.max = schema.maximum;
	}
	if (optional) input.optional = true;
	return input;
}

function extractEvents(
	section: ResolvedSection,
	schemaFor: (name: string) => JsonObject,
	checker: ts.TypeChecker,
): Record<string, ContractEvent> {
	const members = sectionMembers(section, "Events");
	if (members.length === 0) return {};
	const needsSchema = members.some((m) => !isVoidLike(m.typeNode));
	const schema = needsSchema ? schemaFor(section.name) : {};
	const properties = schemaProperties(schema);
	const definitions = schemaDefinitions(schema);

	const events: Record<string, ContractEvent> = {};
	for (const member of members) {
		const description = memberDescription(member.node);
		if (isVoidLike(member.typeNode)) {
			events[member.name] = {
				payloadSchema: null,
				...(description !== undefined && { description }),
			};
			continue;
		}
		const propertySchema = properties[member.name];
		if (propertySchema === undefined) {
			throw new Error(
				`No payload schema was generated for event '${member.name}'`,
			);
		}
		let payloadSchema = inlineRefs(
			propertySchema,
			definitions,
			[],
			`event '${member.name}'`,
		);
		if (
			description !== undefined &&
			isJsonObject(payloadSchema) &&
			payloadSchema.description === description
		) {
			const { description: _lifted, ...rest } = payloadSchema;
			payloadSchema = rest;
		}
		events[member.name] = {
			payloadSchema: requireSchemaObject(
				payloadSchema,
				`event '${member.name}'`,
			),
			...(description !== undefined && { description }),
		};
	}
	return events;
}

function extractQueries(
	section: ResolvedSection,
	schemaFor: (name: string) => JsonObject,
	checker: ts.TypeChecker,
): Record<string, ContractQuery> {
	const members = sectionMembers(section, "Queries");
	if (members.length === 0) return {};
	const schema = schemaFor(section.name);
	const properties = schemaProperties(schema);
	const definitions = schemaDefinitions(schema);

	const queries: Record<string, ContractQuery> = {};
	for (const member of members) {
		const shape = queryShape(member, checker);
		const description = memberDescription(member.node);
		const mutation = memberHasTag(member.node, "mutation");
		const inlined = inlineRefs(
			properties[member.name] ?? {},
			definitions,
			[],
			`query '${member.name}'`,
		);
		const queryProperties = isJsonObject(inlined)
			? schemaProperties(inlined)
			: {};

		const argsSchema = shape.args === null ? null : queryProperties.args;
		if (argsSchema === undefined) {
			throw new Error(
				`No args schema was generated for query '${member.name}'`,
			);
		}
		const resultSchema =
			shape.returns === null ? null : queryProperties.returns;
		if (resultSchema === undefined) {
			throw new Error(
				`No result schema was generated for query '${member.name}'`,
			);
		}

		queries[member.name] = {
			argsSchema:
				argsSchema === null
					? null
					: requireSchemaObject(argsSchema, `query '${member.name}' args`),
			resultSchema:
				resultSchema === null
					? null
					: requireSchemaObject(resultSchema, `query '${member.name}' result`),
			...(description !== undefined && { description }),
			...(mutation && { mutation: true }),
		};
	}
	return queries;
}

interface QueryShape {
	/** `null` when `args: void` (or the member is missing) */
	args: ts.TypeNode | null;
	/** `null` when `returns: void` */
	returns: ts.TypeNode | null;
}

function queryShape(
	member: SectionMember,
	checker: ts.TypeChecker,
): QueryShape {
	let literal: ts.TypeLiteralNode | null = null;
	const typeNode = member.typeNode;
	if (ts.isTypeLiteralNode(typeNode)) {
		literal = typeNode;
	} else if (ts.isTypeReferenceNode(typeNode)) {
		const declaration = resolveTypeDeclaration(typeNode.typeName, checker);
		if (declaration && ts.isTypeAliasDeclaration(declaration)) {
			const aliased = unwrapTypeNode(declaration.type);
			if (ts.isTypeLiteralNode(aliased)) literal = aliased;
		} else if (declaration && ts.isInterfaceDeclaration(declaration)) {
			return interfaceQueryShape(member.name, declaration);
		}
	}
	if (!literal) {
		throw new Error(
			`Query '${member.name}' must be declared as \`{ args: ...; returns: ... }\``,
		);
	}
	return literalQueryShape(member.name, literal.members);
}

function interfaceQueryShape(
	queryName: string,
	declaration: ts.InterfaceDeclaration,
): QueryShape {
	return literalQueryShape(queryName, declaration.members);
}

function literalQueryShape(
	queryName: string,
	members: ts.NodeArray<ts.TypeElement>,
): QueryShape {
	let args: ts.TypeNode | null = null;
	let returns: ts.TypeNode | null | undefined;
	for (const member of members) {
		if (!ts.isPropertySignature(member) || !ts.isIdentifier(member.name)) {
			continue;
		}
		if (!member.type) continue;
		const type = unwrapTypeNode(member.type);
		if (member.name.text === "args") {
			args = isVoidLike(type) ? null : type;
		} else if (member.name.text === "returns") {
			returns = isVoidLike(type) ? null : type;
		}
	}
	if (returns === undefined) {
		throw new Error(
			`Query '${queryName}' must declare a 'returns' member (\`{ args: ...; returns: ... }\`)`,
		);
	}
	return { args, returns };
}

function evaluateObjectLiteral(
	obj: ts.ObjectLiteralExpression,
	path: string,
): JsonObject {
	const out: JsonObject = {};
	for (const property of obj.properties) {
		if (!ts.isPropertyAssignment(property)) {
			throw new Error(
				`Widget config${path ? ` property '${path}'` : ""} may only contain plain \`key: value\` literal assignments (no spreads, shorthands, or methods)`,
			);
		}
		const name =
			ts.isIdentifier(property.name) || ts.isStringLiteral(property.name)
				? property.name.text
				: null;
		if (name === null) {
			throw new Error(
				`Widget config${path ? ` property '${path}'` : ""} contains a computed property name`,
			);
		}
		const propertyPath = path ? `${path}.${name}` : name;
		out[name] = evaluateExpression(property.initializer, propertyPath);
	}
	return out;
}

function evaluateExpression(expr: ts.Expression, path: string): JsonValue {
	const e = unwrapExpression(expr);
	if (ts.isStringLiteral(e) || ts.isNoSubstitutionTemplateLiteral(e)) {
		return e.text;
	}
	if (ts.isNumericLiteral(e)) return Number(e.text);
	if (
		ts.isPrefixUnaryExpression(e) &&
		e.operator === ts.SyntaxKind.MinusToken &&
		ts.isNumericLiteral(e.operand)
	) {
		return -Number(e.operand.text);
	}
	if (e.kind === ts.SyntaxKind.TrueKeyword) return true;
	if (e.kind === ts.SyntaxKind.FalseKeyword) return false;
	if (e.kind === ts.SyntaxKind.NullKeyword) return null;
	if (ts.isArrayLiteralExpression(e)) {
		return e.elements.map((element, index) =>
			evaluateExpression(element, `${path}[${index}]`),
		);
	}
	if (ts.isObjectLiteralExpression(e)) {
		return evaluateObjectLiteral(e, path);
	}
	throw new Error(
		`Widget config property '${path}' must be a literal (string, number, boolean, null, array, or object); computed expressions are not supported`,
	);
}

function readWidgetConfig(
	cfg: JsonObject,
	path: string,
): ExtractedWidgetConfig {
	const id = cfg.id;
	if (typeof id !== "string" || id.length === 0) {
		throw new Error(`Widget config in ${path} must declare a string 'id'`);
	}
	const name = cfg.name;
	if (typeof name !== "string" || name.length === 0) {
		throw new Error(`Widget config in ${path} must declare a string 'name'`);
	}
	const description =
		typeof cfg.description === "string" ? cfg.description : "";

	let sizing: WidgetSizingConfig | undefined;
	if (cfg.sizing !== undefined) {
		if (!isJsonObject(cfg.sizing)) {
			throw new Error(`Widget config 'sizing' in ${path} must be an object`);
		}
		sizing = {};
		const { defaultHeight, resizable, maxHeight } = cfg.sizing;
		if (defaultHeight !== undefined) {
			if (typeof defaultHeight !== "number") {
				throw new Error(`'sizing.defaultHeight' in ${path} must be a number`);
			}
			sizing.defaultHeight = defaultHeight;
		}
		if (resizable !== undefined) {
			if (typeof resizable !== "boolean") {
				throw new Error(`'sizing.resizable' in ${path} must be a boolean`);
			}
			sizing.resizable = resizable;
		}
		if (maxHeight !== undefined) {
			if (typeof maxHeight !== "number") {
				throw new Error(`'sizing.maxHeight' in ${path} must be a number`);
			}
			sizing.maxHeight = maxHeight;
		}
	}

	let capabilities: WidgetCapabilities | undefined;
	if (cfg.capabilities !== undefined) {
		if (!isJsonObject(cfg.capabilities))
			throw new Error(`Widget capabilities in ${path} must be an object`);
		capabilities = {};
		for (const [key, value] of Object.entries(cfg.capabilities)) {
			if (
				!["workers", "media", "microphone", "downloads", "wasm"].includes(
					key,
				) ||
				typeof value !== "boolean"
			) {
				throw new Error(`Invalid widget capability '${key}' in ${path}`);
			}
			capabilities[key as keyof WidgetCapabilities] = value;
		}
	}

	const csp =
		cfg.csp === undefined ? undefined : readWidgetCsp(cfg.csp, id, path);

	let fixtures: Record<string, JsonValue> | undefined;
	if (isJsonObject(cfg.dev) && cfg.dev.fixtures !== undefined) {
		if (!isJsonObject(cfg.dev.fixtures)) {
			throw new Error(
				`Widget config 'dev.fixtures' in ${path} must be an object`,
			);
		}
		fixtures = cfg.dev.fixtures;
	}

	return {
		id,
		name,
		description,
		...(sizing !== undefined && { sizing }),
		...(capabilities !== undefined && { capabilities }),
		...(csp !== undefined && { csp }),
		...(fixtures !== undefined && { fixtures }),
	};
}

const NETWORK_INPUT_KEYS = ["path", "directives", "template"];
const TEMPLATE_KEYS = ["subdomains", "subdomainsInput"];

function invalidCsp(
	kind: string,
	value: JsonValue | undefined,
	location: string,
	id: string,
	reason: string,
): Error {
	return new Error(
		`Invalid widget csp ${kind} ${JSON.stringify(value)} in ${location} for widget ${id}: ${reason}`,
	);
}

function invalidCspShape(
	location: string,
	id: string,
	expectation: string,
): Error {
	return new Error(
		`Invalid widget csp ${location} for widget ${id}: must be ${expectation}`,
	);
}

function rejectUnknownKeys(
	value: JsonObject,
	allowed: readonly string[],
	location: string,
	id: string,
): void {
	for (const key of Object.keys(value)) {
		if (!allowed.includes(key)) {
			throw invalidCsp(
				"key",
				key,
				location,
				id,
				`allowed keys are ${allowed.join(", ")}`,
			);
		}
	}
}

function readStringList(
	value: JsonValue,
	location: string,
	id: string,
	kind: string,
): string[] {
	if (!Array.isArray(value)) {
		throw invalidCspShape(location, id, "an array of string literals");
	}
	return value.map((entry) => {
		if (typeof entry !== "string") {
			throw invalidCsp(kind, entry, location, id, "must be a string");
		}
		return entry;
	});
}

function readCspSources(
	value: JsonValue,
	directive: WidgetCspDirective,
	location: string,
	id: string,
): string[] {
	return readStringList(value, location, id, "source").map((source) => {
		const normalized = normalizeCspSource(source);
		const rejection = validateCspSource(directive, normalized);
		if (rejection !== null) {
			throw invalidCsp(
				"source",
				source,
				location,
				id,
				CSP_SOURCE_REJECTION_MESSAGES[rejection],
			);
		}
		if (validateWildcardBases([normalized]).length > 0) {
			throw invalidCsp(
				"source",
				source,
				location,
				id,
				WILDCARD_PUBLIC_SUFFIX_MESSAGE,
			);
		}
		return source;
	});
}

function readUrlTemplate(
	value: JsonValue,
	location: string,
	id: string,
): WidgetUrlTemplate {
	if (!isJsonObject(value)) {
		throw invalidCspShape(
			location,
			id,
			"an object ({ subdomains?, subdomainsInput? })",
		);
	}
	rejectUnknownKeys(value, TEMPLATE_KEYS, location, id);
	const template: WidgetUrlTemplate = {};
	if (value.subdomains !== undefined) {
		template.subdomains = readStringList(
			value.subdomains,
			`${location}.subdomains`,
			id,
			"subdomain",
		);
	}
	if (value.subdomainsInput !== undefined) {
		if (typeof value.subdomainsInput !== "string") {
			throw invalidCsp(
				"subdomainsInput",
				value.subdomainsInput,
				`${location}.subdomainsInput`,
				id,
				"must be a string naming a widget input",
			);
		}
		template.subdomainsInput = value.subdomainsInput;
	}
	return template;
}

function readNetworkInput(
	value: JsonValue,
	location: string,
	id: string,
): WidgetNetworkInput {
	if (!isJsonObject(value)) {
		throw invalidCspShape(
			location,
			id,
			"an object ({ path, directives, template? })",
		);
	}
	rejectUnknownKeys(value, NETWORK_INPUT_KEYS, location, id);
	if (typeof value.path !== "string") {
		throw invalidCsp(
			"input path",
			value.path,
			`${location}.path`,
			id,
			"must be a string such as tileUrl or layers[].url",
		);
	}
	const directives = readStringList(
		value.directives ?? null,
		`${location}.directives`,
		id,
		"directive",
	).map((directive) => {
		if (!isCspDirective(directive)) {
			throw invalidCsp(
				"directive",
				directive,
				`${location}.directives`,
				id,
				`only ${CSP_DIRECTIVES.join(", ")} can be extended`,
			);
		}
		return directive;
	});
	return {
		path: value.path,
		directives,
		...(value.template !== undefined && {
			template: readUrlTemplate(value.template, `${location}.template`, id),
		}),
	};
}

function readCspPurpose(
	value: JsonValue,
	location: string,
	id: string,
): WidgetCspPurpose {
	if (!isJsonObject(value)) {
		throw invalidCspShape(
			location,
			id,
			"an object ({ reason, connectSrc?, imgSrc?, fontSrc?, mediaSrc?, styleSrc?, inputs? })",
		);
	}
	for (const key of Object.keys(value)) {
		if (!(CSP_PURPOSE_KEYS as readonly string[]).includes(key)) {
			throw invalidCsp(
				"key",
				key,
				location,
				id,
				`only reason, inputs and the directives ${CSP_DIRECTIVES.join(", ")} are allowed; scripts, frames and workers stay limited to the bundle`,
			);
		}
	}
	if (typeof value.reason !== "string") {
		throw invalidCspShape(
			location,
			id,
			"an object with a string 'reason' that tells the viewer why the widget needs these sources",
		);
	}
	const purpose: WidgetCspPurpose = { reason: value.reason };
	for (const directive of CSP_DIRECTIVES) {
		const sources = value[directive];
		if (sources === undefined) continue;
		purpose[directive] = readCspSources(
			sources,
			directive,
			`${location}.${directive}`,
			id,
		);
	}
	if (value.inputs !== undefined) {
		if (!Array.isArray(value.inputs)) {
			throw invalidCspShape(
				`${location}.inputs`,
				id,
				"an array of { path, directives, template? }",
			);
		}
		purpose.inputs = value.inputs.map((input, index) =>
			readNetworkInput(input, `${location}.inputs[${index}]`, id),
		);
	}
	return purpose;
}

function checkCspReasons(purposes: readonly WidgetCspPurpose[], id: string) {
	const owners = new Map<string, number>();
	purposes.forEach((purpose, index) => {
		const location = `csp[${index}].reason`;
		const rejection =
			validateWidgetCspReason(
				purpose.reason,
				(purpose.inputs?.length ?? 0) > 0,
			) ??
			(reasonContainsAddress(purpose.reason)
				? "reason-contains-address"
				: null);
		if (rejection !== null) {
			throw invalidCsp(
				"reason",
				purpose.reason,
				location,
				id,
				cspReasonProblem(rejection),
			);
		}
		const folded = foldWidgetCspReason(purpose.reason);
		const first = owners.get(folded);
		if (first !== undefined) {
			throw invalidCsp(
				"reason",
				purpose.reason,
				location,
				id,
				`${cspReasonProblem("reason-duplicate")}; csp[${first}].reason reads the same`,
			);
		}
		owners.set(folded, index);
	});
}

/**
 * Reads `csp` as an array of purpose groups made of string literals,
 * normalizes it (reasons NFC with collapsed whitespace, sources lowercased
 * and punycoded, lists sorted and deduplicated, inputs sorted by path) and
 * rejects sources, wildcard bases and reasons the hub publish would refuse.
 */
function readWidgetCsp(
	value: JsonValue,
	id: string,
	path: string,
): WidgetCspPurpose[] | undefined {
	if (!Array.isArray(value)) {
		throw invalidCspShape(
			`in ${path}`,
			id,
			'an array of purpose groups, e.g. csp: [{ reason: "Loads map tiles from MapTiler", connectSrc: ["https://api.maptiler.com"] }]',
		);
	}
	const purposes = normalizeCspPurposes(
		value.map((purpose, index) => readCspPurpose(purpose, `csp[${index}]`, id)),
	);
	checkCspReasons(purposes, id);
	return purposes.length > 0 ? purposes : undefined;
}

interface NetworkInputSlot {
	purpose: number;
	input: WidgetNetworkInput;
}

function networkInputSlots(contract: WidgetContract): NetworkInputSlot[] {
	return (contract.csp ?? []).flatMap((purpose, index) =>
		(purpose.inputs ?? []).map((input) => ({ purpose: index, input })),
	);
}

function schemaBranches(schema: JsonObject): JsonObject[] {
	const branches = [schema];
	for (const key of ["anyOf", "oneOf", "allOf"]) {
		const list = schema[key];
		if (!Array.isArray(list)) continue;
		for (const branch of list) {
			if (isJsonObject(branch)) branches.push(...schemaBranches(branch));
		}
	}
	return branches;
}

function schemaHasType(schema: JsonObject, type: string): boolean {
	return (
		schema.type === type ||
		(Array.isArray(schema.type) && schema.type.includes(type))
	);
}

function schemaValueChildren(schema: JsonObject): JsonObject[] {
	const children: JsonObject[] = [];
	if (isJsonObject(schema.additionalProperties)) {
		children.push(schema.additionalProperties);
	}
	if (isJsonObject(schema.patternProperties)) {
		children.push(
			...Object.values(schema.patternProperties).filter(isJsonObject),
		);
	}
	return children;
}

function schemaChildren(
	schema: JsonObject,
	segment: WidgetInputPathSegment,
): JsonObject[] {
	switch (segment.kind) {
		case "key": {
			const property = schemaProperties(schema)[segment.key];
			return isJsonObject(property) ? [property] : schemaValueChildren(schema);
		}
		case "items": {
			if (!schemaHasType(schema, "array")) return [];
			const items = [schema.items, schema.prefixItems].flatMap((entry) =>
				Array.isArray(entry) ? entry : [entry],
			);
			return items.filter(isJsonObject);
		}
		case "values":
			return schemaValueChildren(schema);
	}
}

function reachesString(
	schema: JsonObject,
	segments: readonly WidgetInputPathSegment[],
): boolean {
	const [segment, ...rest] = segments;
	return schemaBranches(schema).some((branch) =>
		segment === undefined
			? schemaHasType(branch, "string")
			: schemaChildren(branch, segment).some((child) =>
					reachesString(child, rest),
				),
	);
}

/**
 * Bundler-only slot check (§14.2.3 rule 5): each network input path must
 * resolve to `type: string` through the generated input schema, with `[]` on
 * arrays and `.*` on objects with `additionalProperties` or
 * `patternProperties`. Roots the contract rules reject are skipped.
 */
export function networkInputSchemaErrors(contract: WidgetContract): string[] {
	const errors: string[] = [];
	for (const { purpose, input } of networkInputSlots(contract)) {
		const parsed = parseWidgetInputPath(input.path);
		if (parsed === null) continue;
		const inputs = contract.inputs ?? {};
		const root = Object.hasOwn(inputs, parsed.root)
			? inputs[parsed.root]
			: undefined;
		if (root === undefined) continue;
		const schema: JsonObject =
			root.type === "string" ? { type: "string" } : (root.schema ?? {});
		if (
			(root.type === "string" || root.type === "json") &&
			!reachesString(schema, parsed.segments)
		) {
			errors.push(
				`Widget '${contract.id}': csp purpose ${purpose}: input "${input.path}" does not reach a string through the schema of input "${parsed.root}" ("[]" needs an array, ".*" an object with additionalProperties or patternProperties)`,
			);
		}
	}
	return errors;
}

function valuesAtPath(
	value: JsonValue | undefined,
	segments: readonly WidgetInputPathSegment[],
): string[] {
	let current: JsonValue[] = value === undefined ? [] : [value];
	for (const segment of segments) {
		current = current.flatMap((entry): JsonValue[] => {
			if (segment.kind === "items") return Array.isArray(entry) ? entry : [];
			if (!isJsonObject(entry)) return [];
			if (segment.kind === "values") return Object.values(entry);
			const child = Object.hasOwn(entry, segment.key)
				? entry[segment.key]
				: undefined;
			return child === undefined ? [] : [child];
		});
	}
	return current.filter((entry): entry is string => typeof entry === "string");
}

const URL_AUTHORITY = /^([a-z][a-z0-9+.-]*):\/\/([^/\\?#]*)/i;

function templateSubdomains(
	contract: WidgetContract,
	template: WidgetUrlTemplate | undefined,
): string[] {
	if (!template) return [];
	if ((template.subdomains?.length ?? 0) > 0) return template.subdomains ?? [];
	const name = template.subdomainsInput;
	const inputs = contract.inputs ?? {};
	const fallback =
		name !== undefined && Object.hasOwn(inputs, name)
			? inputs[name]?.default
			: undefined;
	if (typeof fallback === "string") return Array.from(fallback);
	return Array.isArray(fallback)
		? fallback.filter((entry): entry is string => typeof entry === "string")
		: [];
}

/** Origins a default URL resolves to; `{s}` expands, other placeholders stay. */
function defaultUrlOrigins(
	value: string,
	subdomains: readonly string[],
): string[] {
	const match = URL_AUTHORITY.exec(value.trim());
	const scheme = match?.[1]?.toLowerCase();
	if (!match || (scheme !== "https" && scheme !== "wss")) return [];
	const authority = match[2] ?? "";
	const host = authority
		.slice(authority.lastIndexOf("@") + 1)
		.replace(/:\d*$/, "")
		.replace(/\.$/, "");
	if (host === "" || host.startsWith("[")) return [];
	const hosts =
		host.includes("{s}") && subdomains.length > 0
			? subdomains.map((label) => host.replaceAll("{s}", label))
			: [host];
	return hosts.map((entry) => normalizeCspSource(`${scheme}://${entry}`));
}

function originDeclared(origin: string, sources: readonly string[]): boolean {
	const separator = origin.indexOf("://");
	const scheme = origin.slice(0, separator);
	const host = origin.slice(separator + 3);
	return sources.some((source) => {
		if (source === origin) return true;
		const prefix = `${scheme}://*.`;
		return (
			source.startsWith(prefix) &&
			host.endsWith(`.${source.slice(prefix.length)}`)
		);
	});
}

/**
 * Bundler-only slot lints (§14.2.3 rule 5): an input that feeds `mediaSrc`
 * without `connectSrc`, and a `@default` URL whose origin no static source
 * declares (the SDK merges defaults inside the widget, so the host never
 * approves them).
 */
export function networkInputWarnings(contract: WidgetContract): string[] {
	const warnings: string[] = [];
	const declared = flattenCspPurposes(contract.csp ?? []);
	for (const { purpose, input } of networkInputSlots(contract)) {
		const prefix = `Widget '${contract.id}': csp purpose ${purpose}: input "${input.path}"`;
		if (
			input.directives.includes("mediaSrc") &&
			!input.directives.includes("connectSrc")
		) {
			warnings.push(
				`${prefix} feeds mediaSrc without connectSrc; hls.js and MSE players fetch through connectSrc, native HLS uses mediaSrc`,
			);
		}
		const parsed = parseWidgetInputPath(input.path);
		const inputs = contract.inputs ?? {};
		if (parsed === null || !Object.hasOwn(inputs, parsed.root)) continue;
		const subdomains = templateSubdomains(contract, input.template);
		const origins = new Set(
			valuesAtPath(inputs[parsed.root]?.default, parsed.segments).flatMap(
				(value) => defaultUrlOrigins(value, subdomains),
			),
		);
		for (const origin of origins) {
			const missing = input.directives.filter(
				(directive) => !originDeclared(origin, declared[directive] ?? []),
			);
			if (missing.length === 0) continue;
			warnings.push(
				`${prefix} has a @default URL on ${origin} that no static ${missing.join(", ")} source declares; the SDK merges defaults inside the widget, so the host never sees or approves them. Declare the origin statically or send the URL as an input value`,
			);
		}
	}
	return warnings;
}
