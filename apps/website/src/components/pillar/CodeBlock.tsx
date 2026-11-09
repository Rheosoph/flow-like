import type { ReactNode } from "react";

export type CodeLanguage = "sql" | "cypher" | "flowscript" | "shell" | "json";

const KEYWORDS: Readonly<Record<CodeLanguage, ReadonlySet<string>>> = {
	sql: new Set(
		"select from where join left right inner outer full on as group by order having limit and or not in is null desc asc date case when then else end distinct with union all over partition interval".split(
			" ",
		),
	),
	cypher: new Set(
		"match optional where return order by limit and or not is null as desc asc with unwind create merge set delete detach".split(
			" ",
		),
	),
	flowscript: new Set(
		"use const let function return if else for of in interface true false null await eventsSimple".split(
			" ",
		),
	),
	shell: new Set(["curl"]),
	json: new Set(["true", "false", "null"]),
};

const CASE_INSENSITIVE: ReadonlySet<CodeLanguage> = new Set(["sql", "cypher"]);

const COMMENT: Readonly<Record<CodeLanguage, RegExp | null>> = {
	sql: /^--[^\n]*/,
	cypher: /^\/\/[^\n]*/,
	flowscript: /^\/\/[^\n]*/,
	shell: /^#[^\n]*/,
	json: null,
};

const PATTERNS: readonly [string, RegExp][] = [
	["str", /^("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'|`(?:[^`\\]|\\.)*`)/],
	["num", /^\d+(?:\.\d+)?/],
	["var", /^\$[A-Za-z_]\w*/],
	["word", /^[A-Za-z_][A-Za-z0-9_]*/],
	["space", /^\s+/],
	["op", /^(?:::|->|<-|>=|<=|<>|!=|=>|[^\sA-Za-z0-9_])/],
];

interface Token {
	kind: string;
	text: string;
}

function tokenize(line: string, language: CodeLanguage): Token[] {
	const tokens: Token[] = [];
	let rest = line;
	while (rest.length > 0) {
		const comment = COMMENT[language]?.exec(rest);
		if (comment) {
			tokens.push({ kind: "comment", text: comment[0] });
			break;
		}
		if (language === "shell") {
			const flag = /^-{1,2}[A-Za-z][\w-]*/.exec(rest);
			if (
				flag &&
				(tokens.length === 0 || tokens[tokens.length - 1].kind === "space")
			) {
				tokens.push({ kind: "flag", text: flag[0] });
				rest = rest.slice(flag[0].length);
				continue;
			}
		}
		for (const [kind, pattern] of PATTERNS) {
			const match = pattern.exec(rest);
			if (!match) continue;
			tokens.push({ kind, text: match[0] });
			rest = rest.slice(match[0].length);
			break;
		}
	}
	return tokens;
}

function classify(
	tokens: Token[],
	index: number,
	language: CodeLanguage,
): string {
	const token = tokens[index];
	if (token.kind !== "word") return token.kind;
	const word = CASE_INSENSITIVE.has(language)
		? token.text.toLowerCase()
		: token.text;
	if (KEYWORDS[language].has(word)) return "kw";
	const next = tokens.slice(index + 1).find((item) => item.kind !== "space");
	const previous = tokens
		.slice(0, index)
		.reverse()
		.find((item) => item.kind !== "space");
	if (next?.text === "::") return "ns";
	if (next?.text === "(") return "fn";
	if (language === "cypher" && previous?.text === ":") return "type";
	if (previous?.text === ".") return "prop";
	if (language === "json" && next?.text === ":") return "prop";
	return "id";
}

export default function CodeBlock({
	code,
	language,
	title,
	badge,
	links,
	activeLink,
	onLink,
	className,
}: {
	code: string;
	language: CodeLanguage;
	title?: string;
	badge?: string;
	/** Identifiers that refer to something else on the page, mapped to its id. */
	links?: Readonly<Record<string, string>>;
	activeLink?: string | null;
	onLink?: (id: string | null) => void;
	className?: string;
}) {
	const lines = code.split("\n");
	return (
		<div className={`pl-code ${className ?? ""}`}>
			{title || badge ? (
				<div className="pl-code-bar">
					<span className="pl-code-dots" aria-hidden="true">
						<i />
						<i />
						<i />
					</span>
					{title ? <span className="pl-code-title">{title}</span> : null}
					{badge ? <span className="pl-code-badge">{badge}</span> : null}
				</div>
			) : null}
			<pre>
				<code>
					{lines.map((line, lineIndex) => {
						const tokens = tokenize(line, language);
						const parts: ReactNode[] = tokens.map((token, index) => {
							const kind = classify(tokens, index, language);
							const key = `${lineIndex}-${index}`;
							const link =
								token.kind === "word" ? links?.[token.text] : undefined;
							if (link) {
								return (
									<span
										key={key}
										className="tk-link"
										data-active={activeLink === link}
										onMouseEnter={() => onLink?.(link)}
										onMouseLeave={() => onLink?.(null)}
									>
										{token.text}
									</span>
								);
							}
							return kind === "space" || kind === "id" ? (
								<span key={key}>{token.text}</span>
							) : (
								<span key={key} className={`tk-${kind}`}>
									{token.text}
								</span>
							);
						});
						return (
							<span className="pl-code-line" key={`line-${lineIndex}-${line}`}>
								{parts.length > 0 ? parts : " "}
							</span>
						);
					})}
				</code>
			</pre>
		</div>
	);
}
