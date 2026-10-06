import type {
	CopyValue,
	FieldKey,
	PairRow,
	WorkbenchField,
} from "../contracts";

/**
 * Words that make a field secret when its name or label holds one (spec M6, `FLP_SECRET_WORDS`): the union of
 * every app language (de, en, es, fr, ja, ko, pl, pt-BR, zh), because labels are written in the author's
 * language. Entries with CJK letters match anywhere in the name or label.
 */
export const SECRET_WORDS: readonly string[] = [
	"password",
	"passcode",
	"passphrase",
	"passwd",
	"pwd",
	"secret",
	"token",
	"api key",
	"apikey",
	"access key",
	"private key",
	"client secret",
	"credential",
	"credentials",
	"pin",
	"otp",
	"tan",
	"2fa",
	"mfa",
	"bearer",
	"cookie",
	"session",
	"auth",
	"authorization",
	"iban",
	"card number",
	"cvv",
	"cvc",
	"passwort",
	"kennwort",
	"api schlüssel",
	"zugangsschlüssel",
	"privater schlüssel",
	"geheimnis",
	"zugangsdaten",
	"kartennummer",
	"geheimzahl",
	"mot de passe",
	"clé api",
	"clé d api",
	"clé privée",
	"jeton",
	"identifiants",
	"numéro de carte",
	"contraseña",
	"clave api",
	"clave de api",
	"clave privada",
	"secreto",
	"credenciales",
	"número de tarjeta",
	"senha",
	"chave api",
	"chave de api",
	"chave privada",
	"segredo",
	"credenciais",
	"número do cartão",
	"hasło",
	"klucz api",
	"klucz prywatny",
	"sekret",
	"dane logowania",
	"numer karty",
	"パスワード",
	"暗証番号",
	"トークン",
	"シークレット",
	"apiキー",
	"秘密鍵",
	"비밀번호",
	"암호",
	"토큰",
	"시크릿",
	"api키",
	"비밀키",
	"密码",
	"口令",
	"令牌",
	"密钥",
	"秘钥",
	"卡号",
];

/** Words that cancel a secret word standing directly before or after it in the same phrase ("Max tokens", "Token count", "Session timeout"). */
export const SECRET_EXCLUDE: readonly string[] = [
	"count",
	"max",
	"maximum",
	"min",
	"minimum",
	"limit",
	"length",
	"timeout",
	"ttl",
	"expiry",
	"expires",
	"number of",
	"anzahl",
	"länge",
	"ablauf",
	"nombre de",
	"durée",
	"longueur",
	"limite",
	"número de",
	"duración",
	"longitud",
	"límite",
	"duração",
	"comprimento",
	"liczba",
	"długość",
];

/** Value prefixes of common keys and tokens; a value starting with one is never saved. */
export const SECRET_PREFIXES: readonly string[] = [
	"eyJ",
	"sk-",
	"sk_live_",
	"sk_test_",
	"pk_live_",
	"pk_test_",
	"rk_live_",
	"ghp_",
	"gho_",
	"ghu_",
	"ghs_",
	"github_pat_",
	"glpat-",
	"xoxa-",
	"xoxb-",
	"xoxp-",
	"xoxs-",
	"AIza",
	"-----BEGIN",
];

const CJK_LETTERS = /[\u{3040}-\u{30ff}\u{3400}-\u{9fff}\u{ac00}-\u{d7af}]/u;
const PHRASE_BREAK = /[()[\]{}:;,.!?|/]+/;
const AWS_ACCESS_KEY = /^(AKIA|ASIA)[A-Z0-9]{16}$/;
const KEY_LIKE_RUN = /[A-Za-z0-9_\-+/=]{32,}/g;

/** Combining diacritical marks (U+0300–U+036F) only, so Japanese voicing marks survive the fold. */
const isAccentMark = (char: string) => {
	const code = char.codePointAt(0) ?? 0;
	return code >= 0x300 && code <= 0x36f;
};

/** Lower case without accents, for matching typed text, month words and recent values. */
export function foldText(text: string) {
	const bare = Array.from(text.normalize("NFD"))
		.filter((char) => !isAccentMark(char))
		.join("");
	return bare.normalize("NFC").toLowerCase();
}

/** Words of a name or label in any script: "api_key" → api, key; "accessKey" → access, key. */
function wordsOf(text: string) {
	return text
		.normalize("NFC")
		.replace(/(\p{Ll})(\p{Lu})/gu, "$1 $2")
		.toLowerCase()
		.split(/[^\p{L}\p{N}]+/u)
		.filter(Boolean);
}

/** A label split into phrases at punctuation, so "Password (min 8 characters)" is two phrases. */
function phrasesOf(text: string) {
	return text
		.split(PHRASE_BREAK)
		.map(wordsOf)
		.filter((words) => words.length > 0);
}

const hasCjk = (text: string) => CJK_LETTERS.test(text);

const PHRASE_ENTRIES = SECRET_WORDS.filter((entry) => !hasCjk(entry)).map(
	wordsOf,
);
const CJK_ENTRIES = SECRET_WORDS.filter(hasCjk).map((entry) =>
	foldText(entry).replace(/\s+/g, ""),
);
const EXCLUSIONS = SECRET_EXCLUDE.map(wordsOf);

function standsAt(
	words: readonly string[],
	phrase: readonly string[],
	at: number,
) {
	if (at < 0) return false;
	return phrase.every((word, offset) => words[at + offset] === word);
}

/** The entry's last word may carry a plural ending ("tokens", "passwords"). */
function entryAt(
	words: readonly string[],
	entry: readonly string[],
	at: number,
) {
	const last = entry.length - 1;
	const word = words[at + last];
	const base = entry[last];
	const fits = word === base || word === `${base}s` || word === `${base}es`;
	return fits && standsAt(words, entry.slice(0, last), at);
}

function cancelled(words: readonly string[], at: number, length: number) {
	return EXCLUSIONS.some(
		(phrase) =>
			standsAt(words, phrase, at - phrase.length) ||
			standsAt(words, phrase, at + length),
	);
}

function entryIsSecretIn(words: readonly string[], entry: readonly string[]) {
	for (let at = 0; at + entry.length <= words.length; at++) {
		if (entryAt(words, entry, at) && !cancelled(words, at, entry.length))
			return true;
	}
	return false;
}

function phraseIsSecret(words: readonly string[]) {
	return PHRASE_ENTRIES.some((entry) => entryIsSecretIn(words, entry));
}

/**
 * A field whose values are never saved on this device (spec M6): flagged `sensitive`, named in "Don't save"
 * (`noSave` holds field keys; a top-level key is the field's name), or a secret word in its name or label that
 * no exclusion stands directly before or after in the same phrase. CJK entries match anywhere.
 */
export function isSecretField(
	field: Pick<WorkbenchField, "key" | "name" | "label" | "sensitive">,
	noSave: readonly string[],
) {
	if (field.sensitive || noSave.includes(field.key)) return true;
	const texts = [field.name, field.label];
	if (texts.some((text) => phrasesOf(text).some(phraseIsSecret))) return true;
	const flat = texts
		.map((text) => foldText(text).replace(/\s+/g, ""))
		.join(" ");
	return CJK_ENTRIES.some((entry) => flat.includes(entry));
}

/** A value that looks like a key or token: a known prefix, an AWS key, or a run of 32+ letters, digits, - _ + / = with both letters and digits. */
export function looksSecret(value: unknown) {
	if (typeof value !== "string") return false;
	const text = value.trim();
	if (!text) return false;
	if (SECRET_PREFIXES.some((prefix) => text.startsWith(prefix))) return true;
	if (AWS_ACCESS_KEY.test(text)) return true;
	const runs = text.match(KEY_LIKE_RUN) ?? [];
	return runs.some((run) => /[A-Za-z]/.test(run) && /\d/.test(run));
}

const NO_PROPERTIES: Readonly<Record<string, unknown>> = {};

function recordOf(value: unknown) {
	if (typeof value !== "object" || value === null || Array.isArray(value))
		return null;
	return value as Readonly<Record<string, unknown>>;
}

const isHiddenValue = (value: unknown) => recordOf(value)?.$hidden === true;

const isPairRow = (value: unknown): value is PairRow => {
	const row = recordOf(value);
	return typeof row?.key === "string" && typeof row.value === "string";
};

/** Texts a value carries that could hold a key: the text itself, list entries, keys and values of name/value rows. */
function textsOf(value: unknown) {
	if (typeof value === "string") return [value];
	if (!Array.isArray(value)) return [];
	const texts: string[] = [];
	for (const item of value as readonly unknown[]) {
		if (typeof item === "string") texts.push(item);
		else if (isPairRow(item)) texts.push(item.key, item.value);
	}
	return texts;
}

function keptOut(
	field: WorkbenchField,
	value: unknown,
	noSave: readonly string[],
) {
	return (
		isSecretField(field, noSave) ||
		isHiddenValue(value) ||
		textsOf(value).some(looksSecret)
	);
}

function addProperties(
	hidden: Set<FieldKey>,
	group: WorkbenchField,
	value: unknown,
	noSave: readonly string[],
) {
	if (isSecretField(group, noSave) || isHiddenValue(value)) {
		hidden.add(group.key);
		return;
	}
	const properties = recordOf(value) ?? NO_PROPERTIES;
	for (const prop of group.props) {
		if (keptOut(prop, properties[prop.name], noSave)) hidden.add(prop.key);
	}
}

/**
 * Keys of the fields and object properties whose values stay out of storage: secret fields, "Don't save", and
 * values that look like keys (spec M6). A secret object field is kept out whole; otherwise each property is
 * judged on its own and listed by its own key. Top-level keys are field names.
 */
export const hiddenNames = (
	fields: readonly WorkbenchField[],
	values: Readonly<Record<string, CopyValue>>,
	noSave: readonly string[],
): ReadonlySet<FieldKey> => {
	const hidden = new Set<FieldKey>();
	for (const field of fields) {
		const value = values[field.name];
		if (field.kind === "group") addProperties(hidden, field, value, noSave);
		else if (keptOut(field, value, noSave)) hidden.add(field.key);
	}
	return hidden;
};

/** A short hash of a folded value for `prefs.forgotten` (FNV-1a, 32 bit, 8 hex digits). */
export function hashValue(text: string) {
	const folded = foldText(text.trim());
	let hash = 0x811c9dc5;
	for (let index = 0; index < folded.length; index++) {
		hash ^= folded.charCodeAt(index);
		hash = Math.imul(hash, 0x01000193) >>> 0;
	}
	return hash.toString(16).padStart(8, "0");
}
