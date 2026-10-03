import { i18n as i18next } from "@flow-like/locales";

/* A cron expression in words (no parser): 5 fields, or 6 with a leading seconds field. */

const DOW = [
	"Sunday",
	"Monday",
	"Tuesday",
	"Wednesday",
	"Thursday",
	"Friday",
	"Saturday",
];
const MON = [
	"January",
	"February",
	"March",
	"April",
	"May",
	"June",
	"July",
	"August",
	"September",
	"October",
	"November",
	"December",
];
const pad2 = (n: number) => String(n).padStart(2, "0");
const ord = (n: number) => {
	const s = ["th", "st", "nd", "rd"];
	const v = n % 100;
	return `${n}${s[(v - 20) % 10] || s[v] || s[0]}`;
};
const exact = (s: string) => (/^\d+$/.test(s) ? Number.parseInt(s, 10) : null);
const step = (s: string) => {
	const m = s.match(/^\*\/(\d+)$/);
	return m ? Number.parseInt(m[1], 10) : null;
};
const list = (s: string) =>
	s.includes(",")
		? s
				.split(",")
				.map((x) => Number.parseInt(x, 10))
				.filter(Number.isFinite)
		: null;
const range = (s: string) => {
	const m = s.match(/^(\d+)-(\d+)$/);
	if (!m) return null;
	const a = +m[1];
	const b = +m[2];
	const out: number[] = [];
	for (let i = a; i <= b; i++) out.push(i);
	return out;
};
const joinList = (items: string[]) =>
	items.length <= 1
		? items[0] || ""
		: i18next.t("valAndVal2", "{{val}} and {{val2}}", {
				val: items.slice(0, -1).join(", "),
				val2: items[items.length - 1],
			});
const dowName = (n: number) => DOW[(n === 7 ? 0 : n) % 7] ?? String(n);
const monName = (n: number) => MON[(n - 1) % 12] ?? String(n);
const domToText = (s: string) => {
	const a = list(s) ?? range(s);
	if (a) return joinList(a.map(ord));
	const n = exact(s);
	return n !== null ? ord(n) : null;
};
const monToText = (s: string) => {
	const a = list(s) ?? range(s);
	if (a) return joinList(a.map(monName));
	const n = exact(s);
	return n !== null ? monName(n) : null;
};
const dowToText = (s: string) => {
	if (s === "1-5") return "weekdays";
	if (s === "0,6" || s === "6,0") return "weekends";
	const a = list(s) ?? range(s);
	if (a) return joinList(a.map(dowName));
	const n = exact(s);
	return n !== null ? dowName(n) : null;
};

/** The schedule as a sentence, or null when none of the known shapes fits the expression. */
export function cronWords(expr: string, opts?: { tz?: string }): string | null {
	const tz = opts?.tz ? ` (${opts.tz})` : "";
	if (!expr?.trim()) return null;
	const parts = expr.trim().split(/\s+/);

	// Support both 5-field and 6-field (with seconds) cron
	const [sec, min, hour, dom, mon, dow] =
		parts.length === 6 ? parts : ["0", ...parts];

	const sExact = exact(sec || "*");
	const mExact = exact(min || "*");
	const hExact = exact(hour || "*");
	const mStep = step(min || "*");
	const hStep = step(hour || "*");
	const sStep = step(sec || "*");

	// Fast paths with seconds support
	if (
		sec === "*" &&
		min === "*" &&
		hour === "*" &&
		dom === "*" &&
		mon === "*" &&
		dow === "*"
	)
		return i18next.t("everySecondtz", "Every second{{tz}}", { tz });
	if (
		sStep &&
		min === "*" &&
		hour === "*" &&
		dom === "*" &&
		mon === "*" &&
		dow === "*"
	)
		return i18next.t("everySstepSecondstz", "Every {{sStep}} seconds{{tz}}", {
			sStep,
			tz,
		});
	if (
		sExact !== null &&
		min === "*" &&
		hour === "*" &&
		dom === "*" &&
		mon === "*" &&
		dow === "*"
	)
		return i18next.t(
			"atValOfEveryMinutetz",
			"At :{{val}} of every minute{{tz}}",
			{ val: pad2(sExact), tz },
		);
	if (
		sec === "0" &&
		min === "*" &&
		hour === "*" &&
		dom === "*" &&
		mon === "*" &&
		dow === "*"
	)
		return i18next.t("everyMinutetz", "Every minute{{tz}}", { tz });
	if (
		sec === "0" &&
		mStep &&
		hour === "*" &&
		dom === "*" &&
		mon === "*" &&
		dow === "*"
	)
		return i18next.t("everyMstepMinutestz", "Every {{mStep}} minutes{{tz}}", {
			mStep,
			tz,
		});
	if (
		sec === "0" &&
		mExact !== null &&
		hour === "*" &&
		dom === "*" &&
		mon === "*" &&
		dow === "*"
	)
		return i18next.t(
			"atValPastEveryHourtz",
			"At :{{val}} past every hour{{tz}}",
			{ val: pad2(mExact), tz },
		);
	if (
		sec === "0" &&
		mExact !== null &&
		hExact !== null &&
		dom === "*" &&
		mon === "*" &&
		dow === "*"
	)
		return i18next.t(
			"atValval2EveryDaytz",
			"At {{val}}:{{val2}} every day{{tz}}",
			{ val: pad2(hExact), val2: pad2(mExact), tz },
		);
	if (
		sec === "0" &&
		mExact !== null &&
		hStep &&
		dom === "*" &&
		mon === "*" &&
		dow === "*"
	)
		return i18next.t(
			"everyHstepHoursAtValtz",
			"Every {{hStep}} hours at :{{val}}{{tz}}",
			{ hStep, val: pad2(mExact), tz },
		);

	// With specific seconds
	if (
		sExact !== null &&
		mExact !== null &&
		hExact !== null &&
		dom === "*" &&
		mon === "*" &&
		dow === "*"
	) {
		return i18next.t(
			"atValval2val3EveryDaytz",
			"At {{val}}:{{val2}}:{{val3}} every day{{tz}}",
			{ val: pad2(hExact), val2: pad2(mExact), val3: pad2(sExact), tz },
		);
	}

	// DOW / monthly / monthly+day
	if (
		sec === "0" &&
		mExact !== null &&
		hExact !== null &&
		dom === "*" &&
		mon === "*" &&
		dow !== "*"
	) {
		const when = dowToText(dow);
		if (when)
			return i18next.t(
				"atValval2OnWhentz",
				"At {{val}}:{{val2}} on {{when}}{{tz}}",
				{ val: pad2(hExact), val2: pad2(mExact), when, tz },
			);
	}
	if (
		sec === "0" &&
		mExact !== null &&
		hExact !== null &&
		dom !== "*" &&
		mon === "*" &&
		dow === "*"
	) {
		const days = domToText(dom);
		if (days)
			return i18next.t(
				"atValval2OnTheDaysOfEveryMonthtz",
				"At {{val}}:{{val2}} on the {{days}} of every month{{tz}}",
				{ val: pad2(hExact), val2: pad2(mExact), days, tz },
			);
	}
	if (
		sec === "0" &&
		mExact !== null &&
		hExact !== null &&
		mon !== "*" &&
		dow === "*"
	) {
		const months = monToText(mon);
		if (months) {
			if (dom === "*")
				return i18next.t(
					"atValval2InMonthstz",
					"At {{val}}:{{val2}} in {{months}}{{tz}}",
					{ val: pad2(hExact), val2: pad2(mExact), months, tz },
				);
			const days = domToText(dom);
			if (days)
				return i18next.t(
					"atValval2OnTheDaysOfMonthstz",
					"At {{val}}:{{val2}} on the {{days}} of {{months}}{{tz}}",
					{ val: pad2(hExact), val2: pad2(mExact), days, months, tz },
				);
		}
	}
	if (
		sec === "0" &&
		mExact !== null &&
		hExact !== null &&
		mon !== "*" &&
		dow !== "*"
	) {
		const months = monToText(mon);
		const when = dowToText(dow);
		if (months && when)
			return i18next.t(
				"atValval2OnWhenInMonthstz",
				"At {{val}}:{{val2}} on {{when}} in {{months}}{{tz}}",
				{ val: pad2(hExact), val2: pad2(mExact), when, months, tz },
			);
	}

	return null;
}

export function humanizeCron(expr: string, opts?: { tz?: string }) {
	if (!expr?.trim()) return "";
	return (
		cronWords(expr, opts) ??
		i18next.t("cronExprtz", "Cron: {{expr}}{{tz}}", {
			expr,
			tz: opts?.tz ? ` (${opts.tz})` : "",
		})
	);
}
