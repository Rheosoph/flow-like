import { describe, expect, test } from "bun:test";
import {
	UNSUPPORTED_ACTIVITY_COUNTERS,
	UNSUPPORTED_CONNECTION_STATES,
	UNSUPPORTED_SIZE_ON_DISK,
	UNSUPPORTED_TABLE_SIZES,
	isUnsupported,
} from "./types";

/**
 * The literals are the server's own wording (`DatabaseDetail::unsupported`);
 * a rename on either side must break this test rather than silently turn the
 * detail page back into unexplained empty sections.
 */
const DSQL_UNSUPPORTED = [
	"size on disk",
	"activity counters",
	"connection states",
	"table sizes",
	"dead rows",
];

describe("isUnsupported", () => {
	test("recognizes the statistics a catalog-less engine reports as missing", () => {
		const detail = { unsupported: DSQL_UNSUPPORTED };
		expect(isUnsupported(detail, UNSUPPORTED_CONNECTION_STATES)).toBe(true);
		expect(isUnsupported(detail, UNSUPPORTED_ACTIVITY_COUNTERS)).toBe(true);
		expect(
			isUnsupported(detail, UNSUPPORTED_SIZE_ON_DISK, UNSUPPORTED_TABLE_SIZES),
		).toBe(true);
	});

	test("a Postgres payload without the field keeps every section as it was", () => {
		expect(isUnsupported({}, UNSUPPORTED_SIZE_ON_DISK)).toBe(false);
		expect(isUnsupported({ unsupported: [] }, UNSUPPORTED_SIZE_ON_DISK)).toBe(
			false,
		);
	});

	test("does not report an unrelated statistic as unsupported", () => {
		expect(
			isUnsupported({ unsupported: ["dead rows"] }, UNSUPPORTED_SIZE_ON_DISK),
		).toBe(false);
	});
});
