export interface DeveloperProject {
	id: string;
	path: string;
	language: string;
	name: string;
	createdAt: string;
}

export interface AddProjectInput {
	path: string;
	language: string;
	name: string;
}

export interface DeveloperSettings {
	preferredEditor: string;
	devMode: boolean;
}

export interface ScaffoldInput {
	targetDir: string;
	language: string;
	projectName: string;
}

export type TemplateLanguage =
	| "rust"
	| "python"
	| "typescript"
	| "assemblyscript"
	| "go"
	| "cpp"
	| "csharp"
	| "kotlin"
	| "zig"
	| "nim"
	| "lua"
	| "swift"
	| "java"
	| "grain"
	| "moonbit";

export const TEMPLATE_LANGUAGES: {
	value: TemplateLanguage;
	label: string;
	description: string;
	icon: string;
	img: string;
}[] = [
	{
		value: "rust",
		label: "Rust",
		description: "Best performance, compiled to WASM with Cargo",
		icon: "🦀",
		img: "/lang/rust.jpg",
	},
	{
		value: "python",
		label: "Python",
		description: "Easy to use, compiled via componentize-py",
		icon: "🐍",
		img: "/lang/python.jpg",
	},
	{
		value: "typescript",
		label: "TypeScript",
		description: "Type-safe JavaScript, compiled via ComponentizeJS",
		icon: "🔷",
		img: "/lang/typescript.jpg",
	},
	{
		value: "assemblyscript",
		label: "AssemblyScript",
		description: "TypeScript-like syntax, compiles to WASM natively",
		icon: "📘",
		img: "/lang/assemblyscript.jpg",
	},
	{
		value: "go",
		label: "Go",
		description: "Simple and fast, compiled via TinyGo",
		icon: "🐹",
		img: "/lang/go.jpg",
	},
	{
		value: "cpp",
		label: "C/C++",
		description: "Low-level control, compiled via Emscripten",
		icon: "⚙️",
		img: "/lang/cpp.jpg",
	},
	{
		value: "csharp",
		label: "C#",
		description: ".NET ecosystem, compiled via NativeAOT-WASI",
		icon: "🟣",
		img: "/lang/csharp.jpg",
	},
	{
		value: "kotlin",
		label: "Kotlin",
		description: "JVM ecosystem, compiled via Kotlin/Wasm",
		icon: "🟠",
		img: "/lang/kotlin.jpg",
	},
	{
		value: "zig",
		label: "Zig",
		description: "Systems language, compiles to WASM natively",
		icon: "⚡",
		img: "/lang/zig.jpg",
	},
	{
		value: "nim",
		label: "Nim",
		description: "Expressive language, compiled via Nim → C → Emscripten",
		icon: "👑",
		img: "/lang/nim.jpg",
	},
	{
		value: "lua",
		label: "Lua",
		description: "Lightweight scripting, compiled via Lua → C → Emscripten",
		icon: "🌙",
		img: "/lang/lua.jpg",
	},
	{
		value: "swift",
		label: "Swift",
		description: "Apple ecosystem, compiled via SwiftWasm",
		icon: "🐦",
		img: "/lang/swift.jpg",
	},
	{
		value: "java",
		label: "Java (TeaVM)",
		description: "JVM ecosystem, compiled to WASM via TeaVM",
		icon: "☕",
		img: "/lang/java.jpg",
	},
	{
		value: "grain",
		label: "Grain",
		description: "Functional language, compiles to WASM natively",
		icon: "🌾",
		img: "/lang/grain.jpg",
	},
	{
		value: "moonbit",
		label: "MoonBit",
		description: "Modern language, compiles to WASM natively",
		icon: "🌙",
		img: "/lang/moonbit.jpg",
	},
];

export const EDITOR_OPTIONS = [
	{ value: "vscode", label: "VS Code" },
	{ value: "cursor", label: "Cursor" },
	{ value: "zed", label: "Zed" },
	{ value: "jetbrains", label: "JetBrains IDE" },
	{ value: "fleet", label: "Fleet" },
	{ value: "sublime", label: "Sublime Text" },
	{ value: "nvim", label: "Neovim" },
] as const;

export interface WasmPinDefinition {
	name: string;
	friendly_name: string;
	description: string;
	pin_type: "Input" | "Output";
	data_type: string;
	default_value?: unknown;
	value_type?: string;
	schema?: string;
	valid_values?: string[];
	range?: [number, number];
}

export interface WasmNodeDefinition {
	name: string;
	friendly_name: string;
	description: string;
	category: string;
	icon?: string;
	pins: WasmPinDefinition[];
	scores?: WasmNodeScores;
	long_running?: boolean;
	docs?: string;
}

export interface WasmNodeScores {
	privacy: number;
	security: number;
	performance: number;
	governance: number;
	reliability: number;
	cost: number;
}

export interface WasmExecutionResult {
	outputs: Record<string, unknown>;
	error?: string;
	activate_exec: string[];
	pending?: boolean;
}

export interface RunNodeInput {
	wasmPath: string;
	inputs: Record<string, unknown>;
	nodeName?: string;
}

export interface PackageInspection {
	nodes: WasmNodeDefinition[];
	manifest: import("../wasm").PackageManifest | null;
	isPackage: boolean;
	wasmPath: string;
}
