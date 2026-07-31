import { useMemo, useState } from "react";

export interface CatalogScores {
	privacy: number;
	security: number;
	performance: number;
	governance: number;
	reliability: number;
	cost: number;
}

export interface CatalogPinOptions {
	sensitive?: boolean;
	validValues?: string[];
	range?: [number, number];
	step?: number;
	enforceSchema?: boolean;
	enforceGenericValueType?: boolean;
}

export interface CatalogPin {
	name: string;
	friendlyName: string;
	description: string;
	pinType: "Input" | "Output";
	dataType: string;
	valueType: string;
	schema?: string;
	defaultValue?: unknown;
	index: number;
	options?: CatalogPinOptions;
}

export interface CatalogFnRefs {
	fnRefs: string[];
	canReferenceFns: boolean;
	canBeReferencedByFns: boolean;
}

export interface CatalogNode {
	slug: string;
	packageName: string;
	name: string;
	friendlyName: string;
	description: string;
	category: string;
	categoryPath: string[];
	categorySlug: string;
	icon?: string;
	scores?: CatalogScores;
	pins: CatalogPin[];
	inputCount: number;
	outputCount: number;
	flags: string[];
	version?: number;
	docs?: string;
	oauthProviders: string[];
	requiredOauthScopes: Record<string, string[]>;
	permissions: string[];
	fnRefs?: CatalogFnRefs;
}

export interface CatalogCategory {
	label: string;
	path: string;
	slug: string;
	depth: number;
	count: number;
	description: string;
}

const SCORE_FIELDS: Array<{
	key: keyof CatalogScores;
	label: string;
	help: string;
}> = [
	{
		key: "security",
		label: "Security",
		help: "Attack surface and exposure impact.",
	},
	{
		key: "privacy",
		label: "Privacy",
		help: "Potential sensitivity of processed data.",
	},
	{
		key: "performance",
		label: "Performance",
		help: "Runtime or resource pressure.",
	},
	{
		key: "governance",
		label: "Governance",
		help: "Policy, audit, or compliance impact.",
	},
	{
		key: "reliability",
		label: "Reliability",
		help: "Operational stability considerations.",
	},
	{
		key: "cost",
		label: "Cost",
		help: "External or compute cost impact.",
	},
];

function scoreLabel(score: number): "Low" | "Medium" | "High" {
	if (score >= 7) return "High";
	if (score >= 4) return "Medium";
	return "Low";
}

function scoreClass(score: number): string {
	return `node-score node-score-${scoreLabel(score).toLowerCase()}`;
}

function formatType(pin: CatalogPin): string {
	if (pin.valueType === "Normal") return pin.dataType;
	return `${pin.dataType} ${pin.valueType}`;
}

function formatValue(value: unknown): string {
	if (value === undefined || value === null) return "";
	if (typeof value === "string") return value;
	const serialized = JSON.stringify(value);
	if (!serialized) return String(value);
	return serialized.length > 90 ? `${serialized.slice(0, 87)}...` : serialized;
}

type JsonSchema = Record<string, unknown> & {
	$defs?: Record<string, unknown>;
	$ref?: string;
	additionalProperties?: unknown;
	allOf?: unknown[];
	anyOf?: unknown[];
	const?: unknown;
	default?: unknown;
	definitions?: Record<string, unknown>;
	description?: string;
	enum?: unknown[];
	format?: string;
	items?: unknown;
	oneOf?: unknown[];
	properties?: Record<string, unknown>;
	required?: unknown[];
	title?: string;
	type?: string | string[];
};

function isRecord(value: unknown): value is Record<string, unknown> {
	return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function asSchema(value: unknown): JsonSchema | undefined {
	return isRecord(value) ? (value as JsonSchema) : undefined;
}

function schemaArray(value: unknown): JsonSchema[] {
	if (!Array.isArray(value)) return [];
	return value
		.map(asSchema)
		.filter((schema): schema is JsonSchema => Boolean(schema));
}

function parseSchema(schema?: string): JsonSchema | undefined {
	if (!schema) return undefined;
	const trimmed = schema.trim();
	if (!trimmed.startsWith("{")) return undefined;

	try {
		return asSchema(JSON.parse(trimmed));
	} catch {
		return undefined;
	}
}

function pointerSegment(segment: string): string {
	try {
		return decodeURIComponent(segment).replace(/~1/g, "/").replace(/~0/g, "~");
	} catch {
		return segment.replace(/~1/g, "/").replace(/~0/g, "~");
	}
}

function resolveRef(root: JsonSchema, ref?: string): JsonSchema | undefined {
	if (!ref?.startsWith("#/")) return undefined;
	let current: unknown = root;
	for (const segment of ref.slice(2).split("/").map(pointerSegment)) {
		if (!isRecord(current)) return undefined;
		current = current[segment];
	}
	return asSchema(current);
}

function resolveSchema(
	schema: JsonSchema,
	root: JsonSchema,
	seen = new Set<string>(),
): JsonSchema {
	if (!schema.$ref || seen.has(schema.$ref)) return schema;

	const referenced = resolveRef(root, schema.$ref);
	if (!referenced) return schema;

	seen.add(schema.$ref);
	const { $ref: _ref, ...localSchema } = schema;
	return {
		...resolveSchema(referenced, root, seen),
		...localSchema,
	};
}

function refLabel(ref?: string): string | undefined {
	if (!ref) return undefined;
	const lastSegment = ref.split("/").filter(Boolean).at(-1);
	return lastSegment ? pointerSegment(lastSegment) : undefined;
}

function schemaProperties(schema: JsonSchema, root: JsonSchema) {
	const resolved = resolveSchema(schema, root);
	const direct = isRecord(resolved.properties)
		? (resolved.properties as Record<string, unknown>)
		: undefined;
	const allOf = schemaArray(resolved.allOf);

	if (direct && allOf.length === 0) return direct;

	const merged: Record<string, unknown> = { ...(direct ?? {}) };
	for (const part of allOf) {
		const properties = schemaProperties(part, root);
		if (properties) Object.assign(merged, properties);
	}

	return Object.keys(merged).length > 0 ? merged : undefined;
}

function schemaRequired(schema: JsonSchema, root: JsonSchema): Set<string> {
	const resolved = resolveSchema(schema, root);
	const required = new Set(
		(resolved.required ?? []).filter(
			(field): field is string => typeof field === "string",
		),
	);

	for (const part of schemaArray(resolved.allOf)) {
		for (const field of schemaRequired(part, root)) required.add(field);
	}

	return required;
}

function literalValue(value: unknown): string {
	if (typeof value === "string") return `"${value}"`;
	if (typeof value === "number" || typeof value === "boolean")
		return String(value);
	if (value === null) return "null";
	const serialized = JSON.stringify(value);
	if (!serialized) return String(value);
	return serialized.length > 42 ? `${serialized.slice(0, 39)}...` : serialized;
}

function schemaTypeLabel(schema?: JsonSchema, root?: JsonSchema): string {
	if (!schema) return "value";
	if (schema.$ref && !schema.title) return refLabel(schema.$ref) ?? "reference";

	const base = root ?? schema;
	const resolved = resolveSchema(schema, base);

	if (resolved.title) return resolved.title;
	if (resolved.$ref) return refLabel(resolved.$ref) ?? "reference";
	if (resolved.const !== undefined) return "const";
	if (Array.isArray(resolved.enum) && resolved.enum.length > 0) return "enum";

	const type = resolved.type;
	if (Array.isArray(type)) return type.join(" | ");
	if (type === "array") {
		return `Array<${schemaTypeLabel(asSchema(resolved.items), base)}>`;
	}
	if (type === "object") {
		if (schemaProperties(resolved, base)) return "object";
		const additional = asSchema(resolved.additionalProperties);
		if (additional) return `Map<string, ${schemaTypeLabel(additional, base)}>`;
	}
	if (typeof type === "string")
		return resolved.format ? `${type}:${resolved.format}` : type;

	if (schemaArray(resolved.oneOf).length > 0) {
		return `oneOf (${schemaArray(resolved.oneOf).length})`;
	}
	if (schemaArray(resolved.anyOf).length > 0) {
		return `anyOf (${schemaArray(resolved.anyOf).length})`;
	}
	if (schemaProperties(resolved, base)) return "object";
	if (resolved.items)
		return `Array<${schemaTypeLabel(asSchema(resolved.items), base)}>`;

	return "value";
}

function schemaLabel(schema?: string): string | undefined {
	if (!schema) return undefined;
	const trimmed = schema.trim();
	if (!trimmed.startsWith("{")) return schema;

	const parsed = parseSchema(trimmed);
	if (!parsed) return "JSON schema";

	const resolved = resolveSchema(parsed, parsed);
	if (resolved.title) return resolved.title;
	const properties = schemaProperties(resolved, parsed);
	if (schemaTypeLabel(resolved, parsed) === "object" && properties) {
		return `object (${Object.keys(properties).length} fields)`;
	}
	return schemaTypeLabel(resolved, parsed);
}

function normalize(value: string): string {
	return value.toLowerCase();
}

function slugHref(slug: string): string {
	return `/${slug.toLowerCase()}/`;
}

const CATEGORY_WORD_LABELS: Record<string, string> = {
	ai: "AI",
	api: "API",
	csv: "CSV",
	datafusion: "DataFusion",
	github: "GitHub",
	h3: "H3",
	html: "HTML",
	http: "HTTP",
	imap: "IMAP",
	json: "JSON",
	kg: "KG",
	llm: "LLM",
	mcp: "MCP",
	ml: "ML",
	oauth: "OAuth",
	ocr: "OCR",
	onnx: "ONNX",
	pdf: "PDF",
	rest: "REST",
	smtp: "SMTP",
	sql: "SQL",
	stt: "STT",
	tcp: "TCP",
	tls: "TLS",
	tsv: "TSV",
	tts: "TTS",
	udp: "UDP",
	ui: "UI",
	url: "URL",
	websocket: "WebSocket",
	xai: "xAI",
};

function displayCategoryWord(word: string): string {
	const normalized = word.toLowerCase();
	const known = CATEGORY_WORD_LABELS[normalized];
	if (known) return known;
	if (/[A-Z]/.test(word)) return word;
	return word.charAt(0).toUpperCase() + word.slice(1);
}

function displayCategorySegment(segment: string): string {
	return segment
		.trim()
		.split(/[-_.\s]+/)
		.filter(Boolean)
		.map(displayCategoryWord)
		.join(" ");
}

function displayCategoryPath(category: string): string {
	return category
		.split("/")
		.map(displayCategorySegment)
		.filter(Boolean)
		.join("/");
}

function typeClass(type: string): string {
	return `node-type node-type-${typeToken(type)}`;
}

function typeToken(type: string): string {
	return type.toLowerCase().replace(/[^a-z0-9]+/g, "-");
}

function pinColorClass(type: string): string {
	return `node-pin-color-${typeToken(type)}`;
}

function schemaMeta(schema: JsonSchema, root: JsonSchema): string[] {
	const resolved = resolveSchema(schema, root);
	const meta: string[] = [];

	if (resolved.format) meta.push(`format ${resolved.format}`);
	if (Array.isArray(resolved.enum) && resolved.enum.length > 0) {
		meta.push(
			`enum ${resolved.enum.slice(0, 4).map(literalValue).join(", ")}${
				resolved.enum.length > 4 ? "..." : ""
			}`,
		);
	}
	if (resolved.const !== undefined)
		meta.push(`const ${literalValue(resolved.const)}`);
	if (resolved.default !== undefined)
		meta.push(`default ${literalValue(resolved.default)}`);
	if (typeof resolved.minimum === "number")
		meta.push(`min ${resolved.minimum}`);
	if (typeof resolved.maximum === "number")
		meta.push(`max ${resolved.maximum}`);
	if (typeof resolved.minLength === "number")
		meta.push(`min length ${resolved.minLength}`);
	if (typeof resolved.maxLength === "number")
		meta.push(`max length ${resolved.maxLength}`);

	return meta.slice(0, 4);
}

type SchemaField = {
	name: string;
	role?: string;
	required?: boolean;
	schema: JsonSchema;
};

function schemaFields(schema: JsonSchema, root: JsonSchema): SchemaField[] {
	const resolved = resolveSchema(schema, root);
	const properties = schemaProperties(resolved, root);

	if (properties) {
		const required = schemaRequired(resolved, root);
		return Object.entries(properties)
			.map(([name, value]): SchemaField | undefined => {
				const child = asSchema(value);
				if (!child) return undefined;
				return { name, required: required.has(name), schema: child };
			})
			.filter((field): field is SchemaField => Boolean(field));
	}

	const items = asSchema(resolved.items);
	if (items) return [{ name: "items", role: "array item", schema: items }];

	const additional = asSchema(resolved.additionalProperties);
	if (additional) return [{ name: "*", role: "map value", schema: additional }];

	const variants =
		schemaArray(resolved.oneOf).length > 0
			? schemaArray(resolved.oneOf)
			: schemaArray(resolved.anyOf).length > 0
				? schemaArray(resolved.anyOf)
				: schemaArray(resolved.allOf);

	return variants.map((variant, index) => ({
		name: `variant ${index + 1}`,
		role: "variant",
		schema: variant,
	}));
}

function SchemaFieldTree({
	depth = 0,
	root,
	schema,
}: {
	depth?: number;
	root: JsonSchema;
	schema: JsonSchema;
}) {
	const fields = schemaFields(schema, root);
	const limit = depth === 0 ? 18 : 5;
	const visibleFields = fields.slice(0, limit);

	if (visibleFields.length === 0) {
		return (
			<div className="node-schema-empty">No named fields in this schema.</div>
		);
	}

	return (
		<div className="node-schema-tree">
			{visibleFields.map((field) => {
				const resolved = resolveSchema(field.schema, root);
				const nestedFields =
					depth < 3 ? schemaFields(resolved, root).length > 0 : false;
				const meta = schemaMeta(resolved, root);

				return (
					<div className="node-schema-field" key={`${depth}-${field.name}`}>
						<div className="node-schema-field-main">
							<span className="node-schema-field-name">{field.name}</span>
							<span className="node-schema-field-type">
								{schemaTypeLabel(field.schema, root)}
							</span>
							{field.required ? (
								<span className="node-schema-field-required">required</span>
							) : null}
							{field.role ? (
								<span className="node-schema-field-role">{field.role}</span>
							) : null}
						</div>
						{resolved.description ? (
							<p className="node-schema-field-description">
								{resolved.description}
							</p>
						) : null}
						{meta.length > 0 ? (
							<div className="node-schema-field-meta">
								{meta.map((item) => (
									<span key={item}>{item}</span>
								))}
							</div>
						) : null}
						{nestedFields ? (
							<div className="node-schema-children">
								<SchemaFieldTree
									depth={depth + 1}
									root={root}
									schema={resolved}
								/>
							</div>
						) : null}
					</div>
				);
			})}
			{fields.length > limit ? (
				<div className="node-schema-overflow">
					+{fields.length - limit} more fields
				</div>
			) : null}
		</div>
	);
}

function SchemaPreview({
	label,
	schemaText,
}: {
	label: string;
	schemaText?: string;
}) {
	const parsed = parseSchema(schemaText);
	const title = parsed ? schemaLabel(schemaText) : label;
	const root = parsed ? resolveSchema(parsed, parsed) : undefined;
	const fieldCount = root ? schemaFields(root, root).length : 0;

	return (
		<span className="node-schema-preview">
			<button className="node-schema-trigger" type="button">
				<strong>Schema</strong>
				<span>{label}</span>
			</button>
			<span className="node-schema-popover" role="tooltip">
				<span className="node-schema-popover-header">
					<span>
						<strong>{title}</strong>
						<small>{root ? schemaTypeLabel(root, root) : "Named schema"}</small>
					</span>
					{root ? <span>{fieldCount} fields</span> : null}
				</span>
				{root ? (
					<SchemaFieldTree root={root} schema={root} />
				) : (
					<span className="node-schema-empty">
						Full field metadata is not available for this schema.
					</span>
				)}
			</span>
		</span>
	);
}

function NodeIcon({ node }: { node: CatalogNode }) {
	const [iconFailed, setIconFailed] = useState(false);
	const fallback = node.friendlyName.trim().charAt(0).toUpperCase() || "N";
	const showIcon = Boolean(node.icon && !iconFailed);

	return (
		<div
			className={`node-icon-frame ${showIcon ? "node-icon-frame-has-image" : ""}`}
			aria-hidden="true"
		>
			{showIcon ? (
				<img
					src={node.icon}
					alt=""
					loading="lazy"
					onError={() => setIconFailed(true)}
				/>
			) : null}
			<span>{fallback}</span>
		</div>
	);
}

function ScoreStrip({ scores }: { scores?: CatalogScores }) {
	if (!scores) {
		return (
			<div className="node-score-empty">
				No score metadata has been set for this node yet.
			</div>
		);
	}

	return (
		<div className="node-score-grid" aria-label="Node score ratings">
			{SCORE_FIELDS.map((field) => {
				const score = scores[field.key] ?? 0;
				const label = scoreLabel(score);
				return (
					<div className="node-score-item" key={field.key}>
						<div className="node-score-copy">
							<strong>{field.label}</strong>
							<span>{field.help}</span>
						</div>
						<div className={scoreClass(score)}>
							<span>{score}/10</span>
							<small>{label}</small>
						</div>
					</div>
				);
			})}
		</div>
	);
}

function MetadataPill({
	label,
	value,
}: { label: string; value: string | number }) {
	return (
		<div className="node-meta-pill">
			<span>{label}</span>
			<strong>{value}</strong>
		</div>
	);
}

function PinOptionTags({ pin }: { pin: CatalogPin }) {
	const tags: string[] = [];
	const options = pin.options;
	if (!options) return null;
	if (options.sensitive) tags.push("Sensitive");
	if (options.enforceSchema) tags.push("Schema enforced");
	if (options.enforceGenericValueType) tags.push("Generic type enforced");
	if (options.range)
		tags.push(`Range ${options.range[0]} to ${options.range[1]}`);
	if (options.step !== undefined) tags.push(`Step ${options.step}`);

	if (tags.length === 0 && !options.validValues?.length) return null;

	return (
		<div className="node-pin-options">
			{tags.map((tag) => (
				<span key={tag}>{tag}</span>
			))}
			{options.validValues?.slice(0, 6).map((value) => (
				<span key={value}>{value}</span>
			))}
			{options.validValues && options.validValues.length > 6 ? (
				<span>+{options.validValues.length - 6} more</span>
			) : null}
		</div>
	);
}

function PinList({
	title,
	pins,
	side,
}: {
	title: string;
	pins: CatalogPin[];
	side: "input" | "output";
}) {
	return (
		<section className="node-pin-panel" aria-labelledby={`${side}-pins`}>
			<div className="node-pin-panel-header">
				<h2 id={`${side}-pins`}>{title}</h2>
				<span>{pins.length}</span>
			</div>
			{pins.length === 0 ? (
				<p className="node-empty-copy">No {side} pins.</p>
			) : (
				<div className="node-pin-list">
					{pins.map((pin, position) => {
						const defaultValue = formatValue(pin.defaultValue);
						const schema = schemaLabel(pin.schema);
						return (
							<article
								className="node-pin-row"
								key={`${side}-${pin.index}-${pin.name}-${pin.dataType}-${position}`}
							>
								<div
									className={`node-pin-dot node-pin-dot-${side} ${pinColorClass(
										pin.dataType,
									)}`}
								/>
								<div className="node-pin-body">
									<div className="node-pin-title-row">
										<h3>{pin.friendlyName || pin.name}</h3>
										<span className={typeClass(pin.dataType)}>
											{formatType(pin)}
										</span>
									</div>
									<div className="node-pin-code">{pin.name}</div>
									{pin.description ? <p>{pin.description}</p> : null}
									{schema || defaultValue ? (
										<div className="node-pin-facts">
											{schema ? (
												<SchemaPreview label={schema} schemaText={pin.schema} />
											) : null}
											{defaultValue ? (
												<span>
													<strong>Default</strong> {defaultValue}
												</span>
											) : null}
										</div>
									) : null}
									<PinOptionTags pin={pin} />
								</div>
							</article>
						);
					})}
				</div>
			)}
		</section>
	);
}

function NodeCard({ node }: { node: CatalogNode }) {
	const security = node.scores?.security;
	return (
		<a className="node-card" href={slugHref(node.slug)}>
			<div className="node-card-header">
				<NodeIcon node={node} />
				<div>
					<h3>{node.friendlyName}</h3>
					<span>{displayCategoryPath(node.category)}</span>
				</div>
			</div>
			<p>{node.description}</p>
			<div className="node-card-footer">
				<span>{node.inputCount} in</span>
				<span>{node.outputCount} out</span>
				{security === undefined ? (
					<span className="node-score node-score-unrated">
						Security unrated
					</span>
				) : (
					<span className={scoreClass(security)}>Security {security}/10</span>
				)}
			</div>
		</a>
	);
}

function NodeDirectory({
	nodes,
	title,
}: {
	nodes: CatalogNode[];
	title: string;
}) {
	const [query, setQuery] = useState("");
	const [category, setCategory] = useState("all");
	const [security, setSecurity] = useState("all");

	const categories = useMemo(
		() =>
			Array.from(new Set(nodes.map((node) => node.category)))
				.filter(Boolean)
				.sort((a, b) => a.localeCompare(b)),
		[nodes],
	);

	const filtered = useMemo(() => {
		const needle = normalize(query.trim());
		return nodes.filter((node) => {
			const matchesQuery =
				needle.length === 0 ||
				normalize(node.friendlyName).includes(needle) ||
				normalize(node.name).includes(needle) ||
				normalize(node.description).includes(needle) ||
				normalize(node.category).includes(needle);
			const matchesCategory = category === "all" || node.category === category;
			const securityScore = node.scores?.security;
			const matchesSecurity =
				security === "all" ||
				(security === "unrated"
					? securityScore === undefined
					: securityScore !== undefined &&
						scoreLabel(securityScore).toLowerCase() === security);
			return matchesQuery && matchesCategory && matchesSecurity;
		});
	}, [nodes, query, category, security]);

	return (
		<section className="node-directory" aria-labelledby="node-directory-title">
			<div className="node-section-heading">
				<div>
					<h2 id="node-directory-title">{title}</h2>
					<p>
						Showing {filtered.length} of {nodes.length} generated node docs.
					</p>
				</div>
			</div>
			<search className="node-toolbar">
				<label>
					<span>Search</span>
					<input
						type="search"
						value={query}
						onChange={(event) => setQuery(event.target.value)}
						placeholder="Node name, internal name, category, or description"
					/>
				</label>
				<label>
					<span>Category</span>
					<select
						value={category}
						onChange={(event) => setCategory(event.target.value)}
					>
						<option value="all">All categories</option>
						{categories.map((name) => (
							<option value={name} key={name}>
								{displayCategoryPath(name)}
							</option>
						))}
					</select>
				</label>
				<label>
					<span>Security</span>
					<select
						value={security}
						onChange={(event) => setSecurity(event.target.value)}
					>
						<option value="all">All ratings</option>
						<option value="low">Low exposure</option>
						<option value="medium">Medium exposure</option>
						<option value="high">High exposure</option>
						<option value="unrated">Unrated</option>
					</select>
				</label>
			</search>
			<div className="node-card-grid">
				{filtered.map((node) => (
					<NodeCard node={node} key={node.name} />
				))}
			</div>
		</section>
	);
}

export function NodeReference({ node }: { node?: CatalogNode }) {
	if (!node) {
		return (
			<div className="node-doc-missing not-content">
				This generated node reference could not find its catalog metadata.
			</div>
		);
	}

	const inputs = node.pins.filter((pin) => pin.pinType === "Input");
	const outputs = node.pins.filter((pin) => pin.pinType === "Output");
	const security = node.scores?.security;

	return (
		<div className="node-doc not-content">
			<header className="node-doc-hero">
				<NodeIcon node={node} />
				<div className="node-doc-hero-copy">
					<p className="node-kicker">{displayCategoryPath(node.category)}</p>
					<p>{node.description}</p>
					<div className="node-tag-row">
						<span>{node.name}</span>
						<span>{node.packageName}</span>
						{node.flags.map((flag) => (
							<span key={flag}>{flag}</span>
						))}
					</div>
				</div>
			</header>

			<section className="node-meta-grid" aria-label="Node metadata">
				<MetadataPill label="Inputs" value={node.inputCount} />
				<MetadataPill label="Outputs" value={node.outputCount} />
				<MetadataPill
					label="Security exposure"
					value={security === undefined ? "Unrated" : `${security}/10`}
				/>
				<MetadataPill label="Package" value={node.packageName} />
			</section>

			<section className="node-section">
				<div className="node-section-heading">
					<div>
						<h2>Ratings</h2>
						<p>
							Scores range from 0 to 10. Higher values mean more impact,
							exposure, or operational weight.
						</p>
					</div>
				</div>
				<ScoreStrip scores={node.scores} />
			</section>

			<section className="node-pin-layout" aria-label="Pins">
				<PinList title="Input Pins" pins={inputs} side="input" />
				<PinList title="Output Pins" pins={outputs} side="output" />
			</section>

			<section className="node-details">
				<h2>Node Info</h2>
				<dl>
					<div>
						<dt>Internal name</dt>
						<dd>{node.name}</dd>
					</div>
					<div>
						<dt>Category</dt>
						<dd>{displayCategoryPath(node.category)}</dd>
					</div>
					{node.version !== undefined ? (
						<div>
							<dt>Version</dt>
							<dd>{node.version}</dd>
						</div>
					) : null}
					{node.oauthProviders.length > 0 ? (
						<div>
							<dt>OAuth providers</dt>
							<dd>{node.oauthProviders.join(", ")}</dd>
						</div>
					) : null}
					{node.permissions.length > 0 ? (
						<div>
							<dt>Permissions</dt>
							<dd>{node.permissions.join(", ")}</dd>
						</div>
					) : null}
					{node.fnRefs ? (
						<div>
							<dt>Function references</dt>
							<dd>
								{[
									node.fnRefs.canReferenceFns ? "Can reference functions" : "",
									node.fnRefs.canBeReferencedByFns
										? "Can be referenced by functions"
										: "",
								]
									.filter(Boolean)
									.join(", ") || "No function reference metadata"}
							</dd>
						</div>
					) : null}
				</dl>
			</section>
		</div>
	);
}

export function NodeCategoryOverview({
	category,
	label,
	nodes,
}: {
	category: string;
	label?: string;
	nodes: CatalogNode[];
}) {
	const displayCategory = displayCategoryPath(category);
	const categoryLabel =
		label ??
		displayCategorySegment(
			category.split("/").filter(Boolean).at(-1) ?? category,
		);
	const subcategories = Array.from(new Set(nodes.map((node) => node.category)))
		.filter((name) => name !== category)
		.sort((a, b) => a.localeCompare(b));

	return (
		<div className="node-doc not-content">
			<header className="node-category-hero">
				<p className="node-kicker">{categoryLabel} category</p>
				<p>
					Generated from {nodes.length} catalog node
					{nodes.length === 1 ? "" : "s"} in {displayCategory}.
				</p>
			</header>
			{subcategories.length > 0 ? (
				<section className="node-subcategory-list" aria-label="Subcategories">
					{subcategories.slice(0, 18).map((name) => (
						<span key={name}>{displayCategoryPath(name)}</span>
					))}
					{subcategories.length > 18 ? (
						<span>+{subcategories.length - 18} more</span>
					) : null}
				</section>
			) : null}
			<NodeDirectory nodes={nodes} title="Nodes in this category" />
		</div>
	);
}

export function NodeCatalogOverview({
	nodes,
	categories,
}: {
	nodes: CatalogNode[];
	categories: CatalogCategory[];
}) {
	const topCategories = categories
		.filter((category) => category.depth === 1)
		.sort((a, b) => a.label.localeCompare(b.label));
	const scoredNodes = nodes.filter((node) => node.scores);
	const highSecurityNodes = nodes.filter(
		(node) => (node.scores?.security ?? 0) >= 7,
	).length;

	return (
		<div className="node-doc not-content">
			<header className="node-category-hero">
				<p className="node-kicker">Generated from the native catalog</p>
				<p>
					Browse generated documentation for every built-in Flow-Like node.
					Categories, pins, descriptions, flags, defaults, and available ratings
					come directly from Rust catalog metadata.
				</p>
			</header>

			<section className="node-meta-grid" aria-label="Catalog summary">
				<MetadataPill label="Nodes" value={nodes.length} />
				<MetadataPill label="Categories" value={categories.length} />
				<MetadataPill label="Scored nodes" value={scoredNodes.length} />
				<MetadataPill
					label="High security exposure"
					value={highSecurityNodes}
				/>
			</section>

			<section className="node-section">
				<div className="node-section-heading">
					<div>
						<h2>Categories</h2>
						<p>Top-level groups are generated from node category paths.</p>
					</div>
				</div>
				<div className="node-category-grid">
					{topCategories.map((category) => (
						<a
							className="node-category-card"
							href={slugHref(category.slug)}
							key={category.path}
						>
							<strong>{category.label}</strong>
							<span>{category.count} nodes</span>
							<p>{category.description}</p>
						</a>
					))}
				</div>
			</section>

			<NodeDirectory nodes={nodes} title="All nodes" />
		</div>
	);
}
