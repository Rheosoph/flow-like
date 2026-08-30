/**
 * Minimal Monarch interpreter for tokenizer tests: evaluates the subset of the Monarch
 * grammar that `FLOWSCRIPT_MONARCH` uses (regex rules, group actions, `cases`, `next`/`@pop`,
 * `include`, `@attribute` substitution, default brackets) without loading Monaco.
 */

export interface HarnessToken {
	offset: number;
	type: string;
	text: string;
}

type Action =
	| string
	| Action[]
	| {
			token?: string;
			next?: string;
			cases?: Record<string, Action>;
			bracket?: string;
	  };

type Rule = [RegExp, Action] | { include: string };

interface MonarchLike {
	defaultToken?: string;
	tokenizer: Record<string, Rule[]>;
	[key: string]: unknown;
}

const BRACKETS: Record<string, string> = {
	"{": "delimiter.curly",
	"}": "delimiter.curly",
	"[": "delimiter.square",
	"]": "delimiter.square",
	"(": "delimiter.parenthesis",
	")": "delimiter.parenthesis",
};

function substitute(source: string, def: MonarchLike): string {
	return source.replace(/@(\w+)/g, (_, name: string) => {
		const attr = def[name];
		if (attr instanceof RegExp) return `(?:${attr.source})`;
		if (typeof attr === "string")
			return attr.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
		return `@${name}`;
	});
}

function compileRegex(re: RegExp, def: MonarchLike): RegExp {
	return new RegExp(
		`^(?:${substitute(re.source, def)})`,
		re.flags.replace("g", ""),
	);
}

function expandRules(state: string, def: MonarchLike): [RegExp, Action][] {
	const out: [RegExp, Action][] = [];
	for (const rule of def.tokenizer[state] ?? []) {
		if ("include" in rule) {
			out.push(...expandRules(rule.include.replace(/^@/, ""), def));
		} else {
			out.push([compileRegex(rule[0], def), rule[1]]);
		}
	}
	return out;
}

function resolveCases(
	cases: Record<string, Action>,
	matched: string,
	def: MonarchLike,
): Action {
	for (const [key, action] of Object.entries(cases)) {
		if (key === "@default") continue;
		if (key.startsWith("@")) {
			const list = def[key.slice(1)];
			if (Array.isArray(list) && list.includes(matched)) return action;
		} else if (key === matched) {
			return action;
		}
	}
	return cases["@default"] ?? "";
}

function tokenType(action: Action, matched: string, def: MonarchLike): string {
	if (typeof action === "string") {
		if (action === "@brackets") return BRACKETS[matched] ?? "delimiter.bracket";
		return action;
	}
	if (Array.isArray(action)) return tokenType(action[0], matched, def);
	if (action.cases)
		return tokenType(resolveCases(action.cases, matched, def), matched, def);
	return action.token ?? "";
}

function nextState(
	action: Action,
	matched: string,
	def: MonarchLike,
): string | undefined {
	if (typeof action === "string" || Array.isArray(action)) return undefined;
	if (action.cases)
		return nextState(resolveCases(action.cases, matched, def), matched, def);
	return action.next;
}

/** Tokenizes one line the way Monarch would, merging adjacent tokens of the same type. */
export function tokenizeMonarch(
	def: MonarchLike,
	line: string,
): HarnessToken[] {
	const stack = ["root"];
	const tokens: HarnessToken[] = [];
	let pos = 0;
	const push = (offset: number, type: string, text: string) => {
		const last = tokens[tokens.length - 1];
		if (
			last &&
			last.type === type &&
			last.offset + last.text.length === offset
		) {
			last.text += text;
			return;
		}
		tokens.push({ offset, type, text });
	};
	while (pos < line.length) {
		const rest = line.slice(pos);
		const rules = expandRules(stack[stack.length - 1], def);
		let consumed = false;
		for (const [re, action] of rules) {
			const m = re.exec(rest);
			if (!m || m[0].length === 0) continue;
			if (Array.isArray(action)) {
				let offset = pos;
				for (let g = 0; g < action.length; g++) {
					const text = m[g + 1] ?? "";
					if (text.length > 0)
						push(offset, tokenType(action[g], text, def), text);
					offset += text.length;
				}
			} else {
				push(pos, tokenType(action, m[0], def), m[0]);
			}
			const next = nextState(action, m[0], def);
			if (next === "@pop") stack.pop();
			else if (next === "@push") stack.push(stack[stack.length - 1]);
			else if (next) stack.push(next.replace(/^@/, ""));
			pos += m[0].length;
			consumed = true;
			break;
		}
		if (!consumed) {
			push(pos, def.defaultToken ?? "", line[pos]);
			pos++;
		}
	}
	return tokens;
}
