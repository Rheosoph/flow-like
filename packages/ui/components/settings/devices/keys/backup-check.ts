import type { DevicesT } from "../primitives/area-context";
import type { KeyRow, KeysModel } from "./keys-model";

export interface CheckSummary {
	tone: "good" | "warning";
	text: string;
}

type JoinNames = (list: readonly string[]) => string;

const BEHIND: readonly KeyRow["category"][] = [
	"pending",
	"oldpw",
	"out_of_date",
	"hub_newer",
];
const WITHOUT_BACKUP: readonly KeyRow["category"][] = ["never", "lost"];

function namesIn(
	rows: readonly KeyRow[],
	categories: readonly KeyRow["category"][],
): string[] {
	return rows
		.filter((row) => categories.includes(row.category))
		.map((row) => row.name);
}

/** What differs from this computer, one clause per kind. */
function checkFindings(
	t: DevicesT,
	names: JoinNames,
	rows: readonly KeyRow[],
): string[] {
	const behind = namesIn(rows, BEHIND);
	const restorable = namesIn(rows, ["restorable"]);
	const none = namesIn(rows, WITHOUT_BACKUP);
	return [
		behind.length
			? t("devices:keys.check.behind", {
					count: behind.length,
					devices: names(behind),
					defaultValue_one: "{{devices}} differs from this computer",
					defaultValue_other: "{{devices}} differ from this computer",
				})
			: "",
		restorable.length
			? t("devices:keys.check.restorable", {
					count: restorable.length,
					devices: names(restorable),
					defaultValue_one: "{{devices}} isn't restored here yet",
					defaultValue_other: "{{devices}} aren't restored here yet",
				})
			: "",
		none.length
			? t("devices:keys.check.none", {
					count: none.length,
					devices: names(none),
					defaultValue_one: "{{devices}} has none",
					defaultValue_other: "{{devices}} have none",
				})
			: "",
	].filter(Boolean);
}

/** What "Check backups" found, in one sentence; versions only, nothing is opened. */
export function checkSummary(
	t: DevicesT,
	names: JoinNames,
	model: KeysModel,
	at: string,
): CheckSummary {
	const rows = model.rows.filter(
		(row) => row.relationship !== "cloud_approval",
	);
	const findings = checkFindings(t, names, rows);
	const matching = t("devices:keys.check.matching", {
		count: namesIn(rows, ["synced"]).length,
		defaultValue_one: "{{count, number}} matches this computer",
		defaultValue_other: "{{count, number}} match this computer",
	});
	return {
		tone: findings.length ? "warning" : "good",
		text: t("devices:keys.check.summary", {
			count: rows.filter((row) => row.hubRevision).length,
			at,
			parts: [matching, ...findings].join("; "),
			defaultValue_one:
				"Checked {{count, number}} account backup at {{at}}: {{parts}}.",
			defaultValue_other:
				"Checked {{count, number}} account backups at {{at}}: {{parts}}.",
		}),
	};
}
