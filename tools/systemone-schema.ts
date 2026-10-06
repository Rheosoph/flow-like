type Schema = boolean | Record<string, unknown>;

/** Preserves the native protocol's discriminated unions that quicktype merges. */
export function generateSystemOneTypes(input: unknown): string | undefined {
	if (!input || typeof input !== "object" || Array.isArray(input)) return;
	const schema = input as Record<string, unknown>;
	const title = schema.title;
	if (typeof title !== "string" || !title.startsWith("SystemOne")) return;
	const definitions = (schema.$defs ?? {}) as Record<string, Schema>;
	const external = new Map<string, { name: string; path: string }>();
	if (title === "SystemOneParameters") {
		external.set("ModelProvider", {
			name: "IModelProvider",
			path: "./llm-parameters",
		});
	}
	if (title === "SystemOneRequest") {
		external.set("SystemOneQuestion", {
			name: "ISystemOneQuestion",
			path: "./systemone-question",
		});
	}
	const pending = new Set<string>();
	const imports = new Set<string>();
	const typeName = (name: string) =>
		`I${name.startsWith("SystemOne") ? name : `SystemOne${name}`}`;
	const render = (value: Schema): string => {
		if (value === true) return "unknown";
		if (value === false) return "never";
		if (typeof value.$ref === "string") {
			const match = /^#\/\$defs\/([^/]+)$/.exec(value.$ref);
			if (!match || !Object.hasOwn(definitions, match[1]))
				throw new Error(`Unsupported schema reference: ${value.$ref}`);
			const name = match[1];
			const shared = external.get(name);
			if (shared) {
				imports.add(
					`import type { ${shared.name} } from ${JSON.stringify(shared.path)};`,
				);
				return shared.name;
			}
			pending.add(name);
			return typeName(name);
		}
		if (Object.hasOwn(value, "const")) return JSON.stringify(value.const);
		if (Array.isArray(value.enum))
			return (
				value.enum.map((item) => JSON.stringify(item)).join(" | ") || "never"
			);
		for (const key of ["oneOf", "anyOf"]) {
			if (Array.isArray(value[key]))
				return value[key]
					.map((item) => `(${render(item as Schema)})`)
					.join(" | ");
		}
		if (Array.isArray(value.type))
			return value.type
				.map((type) => `(${render({ ...value, type })})`)
				.join(" | ");
		switch (value.type) {
			case "string":
				return "string";
			case "integer":
			case "number":
				return "number";
			case "boolean":
				return "boolean";
			case "null":
				return "null";
			case "array":
				return `Array<${render((value.items ?? true) as Schema)}>`;
			case "object": {
				const required = new Set((value.required ?? []) as string[]);
				const properties = (value.properties ?? {}) as Record<string, Schema>;
				const fields = Object.entries(properties).map(
					([name, child]) =>
						`${JSON.stringify(name)}${required.has(name) ? "" : "?"}: ${render(child)};`,
				);
				if (value.additionalProperties !== false)
					fields.push(
						`[property: string]: ${render((value.additionalProperties ?? true) as Schema)};`,
					);
				return fields.length
					? `{ ${fields.join(" ")} }`
					: "Record<string, never>";
			}
			default:
				if (
					Object.keys(value).every((key) =>
						["title", "description", "default"].includes(key),
					)
				)
					return "unknown";
				throw new Error(
					`Unsupported SystemOne schema: ${JSON.stringify(value)}`,
				);
		}
	};
	const declarations = [`export type ${typeName(title)} = ${render(schema)};`];
	const rendered = new Set<string>();
	for (const name of pending) {
		if (rendered.has(name)) continue;
		rendered.add(name);
		declarations.push(
			`export type ${typeName(name)} = ${render(definitions[name])};`,
		);
	}
	return `${[...imports, ...declarations].join("\n\n")}\n`;
}
