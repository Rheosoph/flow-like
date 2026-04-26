"use client";

import * as React from "react";

import { formatCodeBlock, isLangSupported } from "@platejs/code-block";
import {
	BracesIcon,
	Check,
	CheckIcon,
	CodeIcon,
	CopyIcon,
	LineChartIcon,
} from "lucide-react";
import { NodeApi, type TCodeBlockElement, type TCodeSyntaxLeaf } from "platejs";
import {
	PlateElement,
	type PlateElementProps,
	PlateLeaf,
	type PlateLeafProps,
	useEditorRef,
	useElement,
	useReadOnly,
} from "platejs/react";

import {
	Button,
	Command,
	CommandEmpty,
	CommandGroup,
	CommandInput,
	CommandItem,
	CommandList,
	Popover,
	PopoverContent,
	PopoverTrigger,
} from "../../..";
import { cn } from "../../../lib/utils";
import { ChartCodeBlock } from "./chart-code-block";

const AdmonitionBlock = React.lazy(() => import("./admonition-block"));
const SpoilerBlock = React.lazy(() => import("./spoiler-block"));
const EmbedCodeBlock = React.lazy(() => import("./embed-code-block"));
const MapCodeBlock = React.lazy(() => import("./map-code-block"));

const CHART_LANGUAGES = ["nivo", "plotly"] as const;
type ChartLanguage = (typeof CHART_LANGUAGES)[number];

function isChartLanguage(lang: string | undefined): lang is ChartLanguage {
	return CHART_LANGUAGES.includes(lang as ChartLanguage);
}

function isDirectiveLanguage(lang: string | undefined): boolean {
	return typeof lang === "string" && lang.startsWith("directive-");
}

function isCustomBlockLanguage(lang: string | undefined): lang is string {
	return lang === "embed" || lang === "map";
}

export function CodeBlockElement(props: PlateElementProps<TCodeBlockElement>) {
	const { editor, element } = props;
	const [showChart, setShowChart] = React.useState(true);
	const lang = element.lang;
	const content = NodeApi.string(element);
	const isChart = isChartLanguage(lang);

	// Render directive blocks (admonition / spoiler) — read-only rendering
	if (isDirectiveLanguage(lang)) {
		const directiveType = lang!.replace("directive-", "");
		const isSpoiler = directiveType === "spoiler";
		return (
			<PlateElement className="py-1" {...props}>
				<React.Suspense
					fallback={
						<div className="h-16 animate-pulse bg-muted/20 rounded-md" />
					}
				>
					{isSpoiler ? (
						<SpoilerBlock content={content} />
					) : (
						<AdmonitionBlock type={directiveType} content={content} />
					)}
				</React.Suspense>
				<div className="hidden">{props.children}</div>
			</PlateElement>
		);
	}

	// Render embed/map custom blocks
	if (isCustomBlockLanguage(lang)) {
		return (
			<PlateElement className="py-1" {...props}>
				<React.Suspense
					fallback={
						<div className="h-16 animate-pulse bg-muted/20 rounded-md" />
					}
				>
					{lang === "embed" ? (
						<EmbedCodeBlock content={content} />
					) : (
						<MapCodeBlock content={content} />
					)}
				</React.Suspense>
				<div className="hidden">{props.children}</div>
			</PlateElement>
		);
	}

	// For chart blocks, show toggle between chart preview and code editor
	if (isChart) {
		return (
			<PlateElement className="codeblock py-1" {...props}>
				<div className="relative rounded-md bg-muted/50">
					{showChart ? (
						<div className="min-h-50">
							<ChartCodeBlock
								content={content}
								language={lang}
								className="rounded-md"
							/>
						</div>
					) : (
						<pre className="overflow-x-auto p-8 pr-4 font-mono text-sm leading-[normal] [tab-size:2] print:break-inside-avoid">
							<code>{props.children}</code>
						</pre>
					)}

					<div
						className="absolute top-1 right-1 z-10 flex gap-0.5 select-none"
						contentEditable={false}
					>
						<Button
							size="icon"
							variant="ghost"
							className="size-6 text-xs"
							onClick={() => setShowChart(!showChart)}
							title={showChart ? "Edit code" : "Show chart"}
						>
							{showChart ? (
								<CodeIcon className="size-3.5! text-muted-foreground" />
							) : (
								<LineChartIcon className="size-3.5! text-muted-foreground" />
							)}
						</Button>

						<CodeBlockCombobox />

						<CopyButton
							size="icon"
							variant="ghost"
							className="size-6 gap-1 text-xs text-muted-foreground"
							value={() => content}
						/>
					</div>
				</div>
			</PlateElement>
		);
	}

	return (
		<PlateElement className="codeblock py-1" {...props}>
			<div className="relative rounded-md bg-muted/50">
				<pre className="overflow-x-auto p-8 pr-4 font-mono text-sm leading-[normal] [tab-size:2] print:break-inside-avoid">
					<code>{props.children}</code>
				</pre>

				<div
					className="absolute top-1 right-1 z-10 flex gap-0.5 select-none"
					contentEditable={false}
				>
					{isLangSupported(element.lang) && (
						<Button
							size="icon"
							variant="ghost"
							className="size-6 text-xs"
							onClick={() => formatCodeBlock(editor, { element })}
							title="Format code"
						>
							<BracesIcon className="size-3.5! text-muted-foreground" />
						</Button>
					)}

					<CodeBlockCombobox />

					<CopyButton
						size="icon"
						variant="ghost"
						className="size-6 gap-1 text-xs text-muted-foreground"
						value={() => NodeApi.string(element)}
					/>
				</div>
			</div>
		</PlateElement>
	);
}

function CodeBlockCombobox() {
	const [open, setOpen] = React.useState(false);
	const readOnly = useReadOnly();
	const editor = useEditorRef();
	const element = useElement<TCodeBlockElement>();
	const value = element.lang || "plaintext";
	const [searchValue, setSearchValue] = React.useState("");

	const items = React.useMemo(
		() =>
			languages.filter(
				(language) =>
					!searchValue ||
					language.label.toLowerCase().includes(searchValue.toLowerCase()),
			),
		[searchValue],
	);

	if (readOnly) return null;

	return (
		<Popover open={open} onOpenChange={setOpen}>
			<PopoverTrigger asChild>
				<Button
					size="sm"
					variant="ghost"
					className="h-6 justify-between gap-1 px-2 text-xs text-muted-foreground select-none"
					aria-expanded={open}
					role="combobox"
				>
					{languages.find((language) => language.value === value)?.label ??
						"Plain Text"}
				</Button>
			</PopoverTrigger>
			<PopoverContent
				className="w-[200px] p-0"
				onCloseAutoFocus={() => setSearchValue("")}
			>
				<Command shouldFilter={false}>
					<CommandInput
						className="h-9"
						value={searchValue}
						onValueChange={(value) => setSearchValue(value)}
						placeholder="Search language..."
					/>
					<CommandEmpty>No language found.</CommandEmpty>

					<CommandList className="h-[344px] overflow-y-auto">
						<CommandGroup>
							{items.map((language) => (
								<CommandItem
									key={language.label}
									className="cursor-pointer"
									value={language.value}
									onSelect={(value) => {
										editor.tf.setNodes<TCodeBlockElement>(
											{ lang: value },
											{ at: element },
										);
										setSearchValue(value);
										setOpen(false);
									}}
								>
									<Check
										className={cn(
											value === language.value ? "opacity-100" : "opacity-0",
										)}
									/>
									{language.label}
								</CommandItem>
							))}
						</CommandGroup>
					</CommandList>
				</Command>
			</PopoverContent>
		</Popover>
	);
}

function CopyButton({
	value,
	...props
}: { value: (() => string) | string } & Omit<
	React.ComponentProps<typeof Button>,
	"value"
>) {
	const [hasCopied, setHasCopied] = React.useState(false);

	React.useEffect(() => {
		setTimeout(() => {
			setHasCopied(false);
		}, 2000);
	}, [hasCopied]);

	return (
		<Button
			onClick={() => {
				void navigator.clipboard.writeText(
					typeof value === "function" ? value() : value,
				);
				setHasCopied(true);
			}}
			{...props}
		>
			<span className="sr-only">Copy</span>
			{hasCopied ? (
				<CheckIcon className="size-3!" />
			) : (
				<CopyIcon className="size-3!" />
			)}
		</Button>
	);
}

export function CodeLineElement(props: PlateElementProps) {
	return <PlateElement {...props} />;
}

export function CodeSyntaxLeaf(props: PlateLeafProps<TCodeSyntaxLeaf>) {
	const tokenClassName = props.leaf.className as string;

	return <PlateLeaf className={tokenClassName} {...props} />;
}

const languages: { label: string; value: string }[] = [
	{ label: "Auto", value: "auto" },
	{ label: "Plain Text", value: "plaintext" },
	// Chart languages (render as interactive charts)
	{ label: "📊 Nivo Chart", value: "nivo" },
	{ label: "📈 Plotly Chart", value: "plotly" },
	// Programming languages
	{ label: "ABAP", value: "abap" },
	{ label: "Agda", value: "agda" },
	{ label: "Arduino", value: "arduino" },
	{ label: "ASCII Art", value: "ascii" },
	{ label: "Assembly", value: "x86asm" },
	{ label: "Bash", value: "bash" },
	{ label: "BASIC", value: "basic" },
	{ label: "BNF", value: "bnf" },
	{ label: "C", value: "c" },
	{ label: "C#", value: "csharp" },
	{ label: "C++", value: "cpp" },
	{ label: "Clojure", value: "clojure" },
	{ label: "CoffeeScript", value: "coffeescript" },
	{ label: "Coq", value: "coq" },
	{ label: "CSS", value: "css" },
	{ label: "Dart", value: "dart" },
	{ label: "Dhall", value: "dhall" },
	{ label: "Diff", value: "diff" },
	{ label: "Docker", value: "dockerfile" },
	{ label: "EBNF", value: "ebnf" },
	{ label: "Elixir", value: "elixir" },
	{ label: "Elm", value: "elm" },
	{ label: "Erlang", value: "erlang" },
	{ label: "F#", value: "fsharp" },
	{ label: "Flow", value: "flow" },
	{ label: "Fortran", value: "fortran" },
	{ label: "Gherkin", value: "gherkin" },
	{ label: "GLSL", value: "glsl" },
	{ label: "Go", value: "go" },
	{ label: "GraphQL", value: "graphql" },
	{ label: "Groovy", value: "groovy" },
	{ label: "Haskell", value: "haskell" },
	{ label: "HCL", value: "hcl" },
	{ label: "HTML", value: "html" },
	{ label: "Idris", value: "idris" },
	{ label: "Java", value: "java" },
	{ label: "JavaScript", value: "javascript" },
	{ label: "JSON", value: "json" },
	{ label: "Julia", value: "julia" },
	{ label: "Kotlin", value: "kotlin" },
	{ label: "LaTeX", value: "latex" },
	{ label: "Less", value: "less" },
	{ label: "Lisp", value: "lisp" },
	{ label: "LiveScript", value: "livescript" },
	{ label: "LLVM IR", value: "llvm" },
	{ label: "Lua", value: "lua" },
	{ label: "Makefile", value: "makefile" },
	{ label: "Markdown", value: "markdown" },
	{ label: "Markup", value: "markup" },
	{ label: "MATLAB", value: "matlab" },
	{ label: "Mathematica", value: "mathematica" },
	{ label: "Mermaid", value: "mermaid" },
	{ label: "Nix", value: "nix" },
	{ label: "Notion Formula", value: "notion" },
	{ label: "Objective-C", value: "objectivec" },
	{ label: "OCaml", value: "ocaml" },
	{ label: "Pascal", value: "pascal" },
	{ label: "Perl", value: "perl" },
	{ label: "PHP", value: "php" },
	{ label: "PowerShell", value: "powershell" },
	{ label: "Prolog", value: "prolog" },
	{ label: "Protocol Buffers", value: "protobuf" },
	{ label: "PureScript", value: "purescript" },
	{ label: "Python", value: "python" },
	{ label: "R", value: "r" },
	{ label: "Racket", value: "racket" },
	{ label: "Reason", value: "reasonml" },
	{ label: "Ruby", value: "ruby" },
	{ label: "Rust", value: "rust" },
	{ label: "Sass", value: "scss" },
	{ label: "Scala", value: "scala" },
	{ label: "Scheme", value: "scheme" },
	{ label: "SCSS", value: "scss" },
	{ label: "Shell", value: "shell" },
	{ label: "Smalltalk", value: "smalltalk" },
	{ label: "Solidity", value: "solidity" },
	{ label: "SQL", value: "sql" },
	{ label: "Swift", value: "swift" },
	{ label: "TOML", value: "toml" },
	{ label: "TypeScript", value: "typescript" },
	{ label: "VB.Net", value: "vbnet" },
	{ label: "Verilog", value: "verilog" },
	{ label: "VHDL", value: "vhdl" },
	{ label: "Visual Basic", value: "vbnet" },
	{ label: "WebAssembly", value: "wasm" },
	{ label: "XML", value: "xml" },
	{ label: "YAML", value: "yaml" },
];
