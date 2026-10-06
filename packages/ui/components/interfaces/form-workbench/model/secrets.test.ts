import { describe, expect, test } from "bun:test";
import {
	type CopyValue,
	FIELD_KEY_SEPARATOR,
	type FieldKind,
	type WorkbenchField,
} from "../contracts";
import {
	SECRET_EXCLUDE,
	SECRET_PREFIXES,
	SECRET_WORDS,
	foldText,
	hashValue,
	hiddenNames,
	isSecretField,
	looksSecret,
} from "./secrets";

function field(
	name: string,
	kind: FieldKind,
	extra: Partial<WorkbenchField> = {},
) {
	const made: WorkbenchField = {
		key: name,
		name,
		label: name,
		help: null,
		kind,
		dataType: "String",
		valueType: "Normal",
		required: false,
		sensitive: false,
		defaultOmitted: false,
		defaultValue: "",
		hasDefault: false,
		options: null,
		range: null,
		step: null,
		integer: false,
		fileMode: null,
		itemKind: null,
		dateFormat: null,
		props: [],
		short: false,
		index: 0,
		...extra,
	};
	return made;
}

const labelled = (label: string) => field("input", "text", { label });
const named = (name: string) => field(name, "text", { label: "Input" });

describe("isSecretField", () => {
	test("the spec's secret labels are secret (M6)", () => {
		const secret = [
			"API key",
			"Password",
			"Auth token",
			"PIN",
			"IBAN",
			"Card number",
			"Password (min 8 characters)",
			"Access token (expires in 1 hour)",
			"API key (max 1 per account)",
		];
		for (const label of secret)
			expect(isSecretField(labelled(label), [])).toBe(true);
	});

	test("labels in every app language are secret for every viewer", () => {
		const secret = [
			"API-Schlüssel",
			"Passwort",
			"Mot de passe",
			"Clé API",
			"Contraseña",
			"Senha",
			"Hasło",
			"パスワード",
			"APIキー",
			"비밀번호",
			"密码",
			"Ihr Passwort bitte",
		];
		for (const label of secret)
			expect(isSecretField(labelled(label), [])).toBe(true);
	});

	test("an exclusion standing directly before or after cancels the word", () => {
		const plain = [
			"Max tokens",
			"Token count",
			"Number of tokens",
			"Session timeout",
			"Author",
			"Customer number",
			"Vendor",
			"Invoice date",
			"Pinned",
			"Spinning",
			"Anzahl Token",
		];
		for (const label of plain)
			expect(isSecretField(labelled(label), [])).toBe(false);
	});

	test("a phrase ends at punctuation, so an exclusion in parentheses does not cancel", () => {
		expect(isSecretField(labelled("Token (max 3)"), [])).toBe(true);
		expect(isSecretField(labelled("Max (token)"), [])).toBe(true);
	});

	test("a guess may mislabel, which only costs the field its history", () => {
		expect(isSecretField(labelled("Session name"), [])).toBe(true);
	});

	test("names are read too: snake case, camel case and plurals", () => {
		expect(isSecretField(named("api_key"), [])).toBe(true);
		expect(isSecretField(named("accessKey"), [])).toBe(true);
		expect(isSecretField(named("client_secret"), [])).toBe(true);
		expect(isSecretField(named("passwords"), [])).toBe(true);
		expect(isSecretField(named("max_tokens"), [])).toBe(false);
		expect(isSecretField(named("vendor_name"), [])).toBe(false);
	});

	test('`sensitive` and "Don\'t save" make any field secret', () => {
		expect(
			isSecretField(field("vendor", "text", { sensitive: true }), []),
		).toBe(true);
		expect(isSecretField(field("vendor", "text"), ["vendor"])).toBe(true);
		expect(isSecretField(field("vendor", "text"), ["other"])).toBe(false);
	});

	test('"Don\'t save" on an object property matches its key, not a field of the same name', () => {
		const key = `address${FIELD_KEY_SEPARATOR}city`;
		const prop = field("city", "text", { key });
		expect(isSecretField(prop, [key])).toBe(true);
		expect(isSecretField(prop, ["city"])).toBe(false);
	});
});

describe("the word lists", () => {
	test("hold every app language", () => {
		const samples = [
			"password",
			"passwort",
			"contraseña",
			"mot de passe",
			"パスワード",
			"비밀번호",
			"hasło",
			"senha",
			"密码",
		];
		for (const word of samples) expect(SECRET_WORDS).toContain(word);
		expect(SECRET_EXCLUDE).toContain("number of");
		expect(SECRET_PREFIXES).toContain("sk-");
	});
});

describe("looksSecret", () => {
	test("known key prefixes, AWS keys and long mixed runs are secret", () => {
		const secret = [
			"sk-proj-abc",
			"sk_live_123",
			"eyJhbGciOiJIUzI1NiJ9",
			"ghp_abcdef",
			"github_pat_11",
			"xoxb-1-2",
			"AIzaSyD",
			"-----BEGIN PRIVATE KEY-----",
			"AKIAABCDEFGHIJKLMNOP",
			"a1".repeat(16),
			`Bearer ${"Ab3".repeat(11)}`,
		];
		for (const value of secret) expect(looksSecret(value)).toBe(true);
	});

	test("everyday values are not", () => {
		const plain = [
			"Nordwind Logistik GmbH",
			"invoice-RE-2026-0918.pdf",
			"https://portal.example.com/orders/48213-7",
			"a".repeat(40),
			"1".repeat(40),
			"",
			"   ",
		];
		for (const value of plain) expect(looksSecret(value)).toBe(false);
	});

	test("only text can look like a key", () => {
		expect(looksSecret(42)).toBe(false);
		expect(looksSecret(null)).toBe(false);
		expect(looksSecret({ value: "sk-1" })).toBe(false);
		expect(looksSecret(["sk-1"])).toBe(false);
	});
});

describe("foldText and hashValue", () => {
	test("folding drops case and accents but keeps Japanese voicing marks", () => {
		expect(foldText("Contraseña")).toBe("contrasena");
		expect(foldText("ÉCOLE")).toBe("ecole");
		expect(foldText("Wrzesień")).toBe("wrzesien");
		expect(foldText("パスワード")).toBe("パスワード");
		expect(foldText("비밀번호")).toBe("비밀번호");
	});

	test("FNV-1a of the folded, trimmed text, as 8 hex digits (same as flpHash)", () => {
		expect(hashValue("Nordwind Logistik GmbH")).toBe("3531b96a");
		expect(hashValue("  nordwind logistik gmbh ")).toBe("3531b96a");
		expect(hashValue("Alpenfracht AG")).toBe("abc1ce39");
		expect(hashValue("")).toBe("811c9dc5");
		expect(hashValue("Contraseña")).toBe("bba0afe1");
		expect(hashValue("パスワード")).toBe("931765f7");
	});
});

describe("hiddenNames", () => {
	const loginProps = [
		field("user", "text", { key: `login${FIELD_KEY_SEPARATOR}user` }),
		field("password", "text", { key: `login${FIELD_KEY_SEPARATOR}password` }),
		field("token_note", "text", {
			key: `login${FIELD_KEY_SEPARATOR}token_note`,
			label: "Note",
		}),
	];
	const fields = [
		field("vendor_name", "text", { label: "Vendor" }),
		field("api_key", "text", { label: "API key" }),
		field("notes", "text", { label: "Notes" }),
		field("max_pages", "number", { label: "Max pages" }),
		field("tags", "chips", { label: "Tags", itemKind: "text" }),
		field("headers", "pairs", { label: "Headers" }),
		field("invoice_file", "file", { label: "Invoice" }),
		field("login", "group", { label: "Login", props: loginProps }),
		field("credentials", "group", {
			label: "Credentials",
			props: [
				field("user", "text", { key: `credentials${FIELD_KEY_SEPARATOR}user` }),
			],
		}),
		field("comment", "text", { label: "Comment" }),
	];
	const values: Record<string, CopyValue> = {
		vendor_name: "Nordwind Logistik GmbH",
		api_key: "anything",
		notes: "sk-live-abc",
		max_pages: "20",
		tags: ["ok", "ghp_abcdef"],
		headers: [
			{ id: "h1", key: "Authorization", value: `Bearer ${"Ab3".repeat(11)}` },
		],
		invoice_file: null,
		login: { user: "felix", password: "hunter2", token_note: "fine" },
		credentials: { user: "felix" },
		comment: { $hidden: true },
	};

	test("secret fields, key-like values and hidden values stay out; object properties by their own key", () => {
		expect([...hiddenNames(fields, values, [])].sort()).toEqual(
			[
				"api_key",
				"comment",
				"credentials",
				"headers",
				`login${FIELD_KEY_SEPARATOR}password`,
				`login${FIELD_KEY_SEPARATOR}token_note`,
				"notes",
				"tags",
			].sort(),
		);
	});

	test('"Don\'t save" adds the field; everyday values stay', () => {
		const hidden = hiddenNames(fields, values, ["vendor_name"]);
		expect(hidden.has("vendor_name")).toBe(true);
		expect(hidden.has("max_pages")).toBe(false);
		expect(hidden.has("invoice_file")).toBe(false);
		expect(hidden.has(`login${FIELD_KEY_SEPARATOR}user`)).toBe(false);
	});

	test("nothing secret, nothing hidden", () => {
		const plain = [field("vendor_name", "text", { label: "Vendor" })];
		expect(hiddenNames(plain, { vendor_name: "Alpenfracht AG" }, []).size).toBe(
			0,
		);
	});
});
