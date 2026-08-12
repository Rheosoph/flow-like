import { existsSync } from "node:fs";
import { resolve } from "node:path";
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
	CONTRACT_VERSION,
	type ContractEvent,
	type ContractInput,
	type ContractQuery,
	type JsonValue,
	type WidgetContract,
	validateContract,
} from "./contract-types";

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

	const generatorConfig: CompletedConfig = {
		...DEFAULT_CONFIG,
		path: absPath,
		skipTypeCheck: true,
		jsDoc: "extended",
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
		contractVersion: CONTRACT_VERSION,
		id: config.id,
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
	if (errors.length > 0) {
		throw new Error(
			`Invalid contract for widget '${config.id}' (${absPath}): ${errors.join("; ")}`,
		);
	}

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
): JsonValue {
	if (Array.isArray(value)) {
		return value.map((item) => inlineRefs(item, definitions, stack, context));
	}
	if (!isJsonObject(value)) return value;

	const { $ref, $schema, definitions: _nested, ...rest } = value;
	void $schema;
	void _nested;

	const inlinedRest: JsonObject = {};
	for (const [key, entry] of Object.entries(rest)) {
		inlinedRest[key] = inlineRefs(entry, definitions, stack, context);
	}

	if (typeof $ref !== "string") return inlinedRest;

	const prefix = "#/definitions/";
	if (!$ref.startsWith(prefix)) {
		throw new Error(
			`Unsupported $ref '${$ref}' while inlining the schema for ${context}`,
		);
	}
	const encoded = $ref.slice(prefix.length);
	const key = decodeURIComponent(encoded);
	const definition = definitions[key] ?? definitions[encoded];
	if (definition === undefined) {
		throw new Error(
			`Unresolvable $ref '${$ref}' while inlining the schema for ${context}`,
		);
	}
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
		if (!optional && input.default === undefined) {
			warnings.push(
				`Input '${member.name}' of widget '${widgetId}' has no @default and is not optional; standalone dev and generated pin defaults will have no value`,
			);
		}
		inputs[member.name] = input;
	}
	return inputs;
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
		...(fixtures !== undefined && { fixtures }),
	};
}
