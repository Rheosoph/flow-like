"use client";

import {
	Braces,
	ChevronDown,
	Plus,
	SlidersHorizontal,
	Trash2,
} from "lucide-react";
import { createContext, useContext, useState } from "react";
import { IBitTypes } from "../../lib/schema/bit/bit";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { Switch } from "../ui/switch";
import { Textarea } from "../ui/textarea";
import { EditorField, EditorSection, StringList } from "./bit-editor-fields";
import { record } from "./bit-editor-model";

const LABELS: Record<string, string> = {
	context_length: "Context length",
	model_classification: "Capabilities",
	provider: "Connection",
	provider_name: "Provider",
	model_id: "Model ID",
	api_surface: "API surface",
	params: "Provider settings",
	vector_length: "Vector dimensions",
	input_length: "Input length",
	api_key: "API key",
	endpoint: "Endpoint URL",
	base_url: "Base URL",
	huggingface: "Hugging Face files",
	sample_rate: "Sample rate",
	function_calling: "Tool calling",
};
const labelFor = (key: string) =>
	LABELS[key] ??
	key.replaceAll("_", " ").replace(/^./, (char) => char.toUpperCase());
const HINTS: Record<string, string> = {
	context_length: "Maximum number of tokens the model can accept.",
	model_id: "The exact model identifier expected by your provider.",
	vector_length: "Number of values in each embedding vector.",
	provider_name: "The service or local engine that runs this model.",
	model_classification: "Capability scores use a scale from 0 to 1.",
	huggingface: "Pinned model files used by the local runtime.",
};
const ProviderScope = createContext<"custom" | "admin">("admin");
const CUSTOM_PROVIDERS = [
	"openai",
	"anthropic",
	"gemini",
	"mistral",
	"groq",
	"openrouter",
	"together",
	"ollama",
	"lmstudio",
	"xai",
	"deepseek",
	"bedrock",
	"vertex",
	"huggingface",
	"cohere",
	"perplexity",
	"moonshot",
	"hyperbolic",
	"galadriel",
	"mira",
	"mozilla",
];
const PROVIDER_LABELS: Record<string, string> = {
	openai: "OpenAI",
	anthropic: "Anthropic",
	gemini: "Google Gemini",
	mistral: "Mistral",
	groq: "Groq",
	openrouter: "OpenRouter",
	together: "Together AI",
	ollama: "Ollama",
	lmstudio: "LM Studio",
	xai: "xAI",
	deepseek: "DeepSeek",
	bedrock: "Amazon Bedrock",
	vertex: "Google Vertex AI",
	huggingface: "Hugging Face",
	cohere: "Cohere",
	perplexity: "Perplexity",
	moonshot: "Moonshot",
	hyperbolic: "Hyperbolic",
	galadriel: "Galadriel",
	mira: "Mira",
	mozilla: "Mozilla",
	azure: "Azure",
};

function ParameterValue({
	name,
	value,
	onChange,
	path,
}: {
	name: string;
	value: unknown;
	onChange: (value: unknown) => void;
	path: string;
}) {
	const scope = useContext(ProviderScope);
	const label = labelFor(name);
	if (name === "provider_name") {
		const current = typeof value === "string" ? value : "";
		const options = [
			{ value: "Local", label: "Local (llama.cpp)" },
			{ value: "MLX", label: "Local (MLX)" },
			...(scope === "custom"
				? CUSTOM_PROVIDERS.map((provider) => ({
						value: `custom:${provider}`,
						label: PROVIDER_LABELS[provider],
					}))
				: [
						{ value: "Hosted", label: "Hosted (automatic)" },
						{ value: "Premium", label: "Hosted (premium)" },
						{ value: "Internal", label: "Hosted (internal)" },
						...[
							"openrouter",
							"openai",
							"anthropic",
							"bedrock",
							"azure",
							"vertex",
						].map((provider) => ({
							value: `hosted:${provider}`,
							label: `Hosted: ${PROVIDER_LABELS[provider]}`,
						})),
					]),
		];
		return (
			<div className="space-y-2">
				<label htmlFor={path} className="text-sm font-medium">
					Provider
				</label>
				<select
					id={path}
					className="h-9 w-full rounded-md border bg-background px-3 text-sm"
					value={current}
					onChange={(e) => onChange(e.target.value)}
				>
					{!options.some((option) => option.value === current) && (
						<option value={current}>{current || "Choose a provider"}</option>
					)}
					{options.map((option) => (
						<option key={option.value} value={option.value}>
							{option.label}
						</option>
					))}
				</select>
				<p className="text-xs text-muted-foreground">{HINTS.provider_name}</p>
			</div>
		);
	}
	if (name === "api_surface")
		return (
			<div className="space-y-2">
				<label htmlFor={path} className="text-sm font-medium">
					API surface
				</label>
				<select
					id={path}
					value={typeof value === "string" ? value : ""}
					onChange={(e) => onChange(e.target.value || null)}
					className="h-9 w-full rounded-md border bg-background px-3 text-sm"
				>
					<option value="">Provider default</option>
					<option value="ChatCompletions">Chat Completions</option>
					<option value="Responses">Responses</option>
				</select>
				<p className="text-xs text-muted-foreground">
					Leave the provider default unless your model requires a specific API.
				</p>
			</div>
		);
	if (typeof value === "boolean")
		return (
			<div className="flex items-center justify-between gap-4 rounded-lg border p-3">
				<label htmlFor={path} className="text-sm font-medium">
					{label}
				</label>
				<Switch id={path} checked={value} onCheckedChange={onChange} />
			</div>
		);
	if (typeof value === "number")
		return (
			<EditorField
				label={label}
				hint={HINTS[name]}
				type="number"
				value={value}
				onChange={(text) => onChange(Number(text))}
			/>
		);
	if (typeof value === "string" || value == null)
		return (
			<EditorField
				label={label}
				hint={HINTS[name]}
				value={value == null ? "" : (value as string)}
				placeholder={value == null ? "Not set" : undefined}
				onChange={(text) => onChange(text || (value == null ? null : ""))}
			/>
		);
	if (Array.isArray(value) && value.every((item) => typeof item === "string"))
		return (
			<StringList
				label={label}
				value={value}
				onChange={onChange}
				placeholder={`Add ${label.toLowerCase()}`}
			/>
		);
	if (Array.isArray(value))
		return (
			<details className="rounded-lg border p-4">
				<summary className="cursor-pointer text-sm font-medium">
					{label}{" "}
					<span className="text-muted-foreground">({value.length})</span>
				</summary>
				<div className="mt-4 space-y-4">
					{value.map((item, index) => (
						<ParameterValue
							// biome-ignore lint/suspicious/noArrayIndexKey: Array entries retain their positions in this editor.
							key={`${path}-${index}`}
							name={`${name} ${index + 1}`}
							path={`${path}-${index}`}
							value={item}
							onChange={(next) =>
								onChange(value.map((old, i) => (i === index ? next : old)))
							}
						/>
					))}
				</div>
			</details>
		);
	return (
		<details
			open={name === "provider" || name === "params" || name === "prefix"}
			className="group rounded-xl border bg-background p-4"
		>
			<summary className="flex cursor-pointer list-none items-center justify-between text-sm font-medium">
				{label}
				<ChevronDown className="size-4 text-muted-foreground" />
			</summary>
			{HINTS[name] && (
				<p className="mt-1 text-xs text-muted-foreground">{HINTS[name]}</p>
			)}
			<div className="mt-5">
				<ParameterObject
					value={record(value)}
					onChange={onChange}
					path={path}
				/>
			</div>
		</details>
	);
}
function ParameterObject({
	value,
	onChange,
	path,
}: {
	value: Record<string, unknown>;
	onChange: (value: unknown) => void;
	path: string;
}) {
	const [adding, setAdding] = useState(false);
	const [key, setKey] = useState("");
	const [type, setType] = useState("text");
	const duplicate = Object.hasOwn(value, key.trim());
	return (
		<div className="space-y-4">
			{Object.entries(value).map(([key, item]) => (
				<div key={key} className="group/field flex items-start gap-2">
					<div className="min-w-0 flex-1">
						<ParameterValue
							name={key}
							value={item}
							path={`${path}-${key}`}
							onChange={(next) => onChange({ ...value, [key]: next })}
						/>
					</div>
					<Button
						disabled={["context_length", "provider", "provider_name"].includes(
							key,
						)}
						variant="ghost"
						size="icon"
						className="mt-6 size-8 shrink-0 text-muted-foreground opacity-60 hover:text-destructive"
						aria-label={`Remove ${labelFor(key)} parameter`}
						onClick={() => {
							const next = { ...value };
							delete next[key];
							onChange(next);
						}}
					>
						<Trash2 className="size-3.5" />
					</Button>
				</div>
			))}
			{path.endsWith("-provider") && !Object.hasOwn(value, "api_surface") && (
				<ParameterValue
					name="api_surface"
					value={null}
					path={`${path}-api_surface`}
					onChange={(next) => onChange({ ...value, api_surface: next })}
				/>
			)}
			{adding ? (
				<div className="space-y-2 rounded-lg border border-dashed p-3">
					<div className="flex flex-wrap gap-2">
						<Input
							aria-label="New parameter name"
							placeholder="parameter_name"
							value={key}
							onChange={(e) => setKey(e.target.value)}
							className="min-w-0 flex-1"
						/>
						<select
							aria-label="New parameter type"
							value={type}
							onChange={(e) => setType(e.target.value)}
							className="rounded-md border bg-background px-2 text-sm"
						>
							<option value="text">Text</option>
							<option value="number">Number</option>
							<option value="boolean">Toggle</option>
							<option value="object">Group</option>
							<option value="list">List</option>
						</select>
						<Button
							className="bg-foreground text-background hover:bg-foreground/90"
							size="sm"
							disabled={
								!key.trim() ||
								duplicate ||
								["__proto__", "constructor", "prototype"].includes(key.trim())
							}
							onClick={() => {
								onChange({
									...value,
									[key.trim()]:
										type === "number"
											? 0
											: type === "boolean"
												? false
												: type === "object"
													? {}
													: type === "list"
														? []
														: "",
								});
								setKey("");
								setAdding(false);
							}}
						>
							Add
						</Button>
						<Button variant="ghost" size="sm" onClick={() => setAdding(false)}>
							Cancel
						</Button>
					</div>
					{duplicate && (
						<p role="alert" className="text-xs text-destructive">
							This parameter already exists.
						</p>
					)}
				</div>
			) : (
				<Button
					variant="ghost"
					size="sm"
					className="text-muted-foreground"
					onClick={() => setAdding(true)}
				>
					<Plus className="size-3.5" />
					Add parameter
				</Button>
			)}
		</div>
	);
}
export function BitParametersEditor({
	value,
	bitType,
	onChange,
	jsonText,
	onJsonChange,
	jsonError,
	onApplyJson,
	onResetJson,
	scope = "admin",
}: {
	value: unknown;
	bitType: IBitTypes;
	onChange: (value: unknown) => void;
	jsonText: string | null;
	onJsonChange: (text: string) => void;
	jsonError: string | null;
	onApplyJson: () => void;
	onResetJson: () => void;
	scope?: "custom" | "admin";
}) {
	const [mode, setMode] = useState<"form" | "json">("form");
	const params = record(value);
	const isModel = [IBitTypes.Llm, IBitTypes.Vlm].includes(bitType);
	return (
		<ProviderScope.Provider value={scope}>
			<EditorSection
				title="Parameters"
				description="Configure how this bit runs. Changes stay in your draft until you save."
			>
				<div className="inline-flex rounded-lg bg-muted p-1">
					<Button
						size="sm"
						variant={mode === "form" ? "secondary" : "ghost"}
						disabled={jsonText !== null}
						onClick={() => setMode("form")}
					>
						<SlidersHorizontal className="size-4" />
						Fields
					</Button>
					<Button
						size="sm"
						variant={mode === "json" ? "secondary" : "ghost"}
						onClick={() => setMode("json")}
					>
						<Braces className="size-4" />
						JSON
					</Button>
				</div>
				{mode === "json" ? (
					<div className="space-y-3">
						<label
							htmlFor="bit-parameters-json"
							className="text-sm font-medium"
						>
							Parameters JSON
						</label>
						<Textarea
							id="bit-parameters-json"
							value={jsonText ?? JSON.stringify(value, null, 2) ?? "null"}
							spellCheck={false}
							className="min-h-80 font-mono text-xs leading-relaxed"
							onChange={(e) => onJsonChange(e.target.value)}
							aria-invalid={!!jsonError}
						/>
						{jsonError && (
							<p role="alert" className="text-sm text-destructive">
								{jsonError}
							</p>
						)}
						<div className="flex gap-2">
							<Button
								className="bg-foreground text-background hover:bg-foreground/90"
								size="sm"
								disabled={jsonText === null}
								onClick={onApplyJson}
							>
								Apply JSON
							</Button>
							<Button
								size="sm"
								variant="ghost"
								disabled={jsonText === null}
								onClick={onResetJson}
							>
								Discard JSON edits
							</Button>
						</div>
						<p className="text-xs text-muted-foreground">
							Apply your JSON to return to fields. Saving is available after it
							has been applied.
						</p>
					</div>
				) : (
					<div className="space-y-5">
						{isModel && !Object.hasOwn(params, "context_length") && (
							<Button
								variant="outline"
								onClick={() => onChange({ ...params, context_length: 4096 })}
							>
								Add context length
							</Button>
						)}
						{typeof value === "object" &&
						value !== null &&
						!Array.isArray(value) ? (
							<ParameterObject
								value={params}
								onChange={onChange}
								path="parameter"
							/>
						) : (
							<div className="rounded-lg border border-dashed p-5 text-sm text-muted-foreground">
								This bit has {value == null ? "no" : "a custom"} parameter
								structure.{" "}
								<Button variant="link" onClick={() => setMode("json")}>
									Edit JSON
								</Button>
								{value == null && (
									<Button variant="outline" onClick={() => onChange({})}>
										Add parameters
									</Button>
								)}
							</div>
						)}
					</div>
				)}
			</EditorSection>
		</ProviderScope.Provider>
	);
}
