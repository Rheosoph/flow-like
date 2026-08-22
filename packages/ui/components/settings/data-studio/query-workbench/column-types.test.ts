import { describe, expect, test } from "bun:test";
import type { QueryColumn } from "../../../../state/backend-state/query-state";
import { classifyColumn, classifyResultColumn } from "./column-types";

const SUB = "42c52474-5081-70d7-2b23-4bd8c38d8fb0";

const column = (name: string, type_name = "Utf8"): QueryColumn =>
	({ name, type_name }) as QueryColumn;

describe("classifyColumn", () => {
	test("reads a text column that names a person as a person", () => {
		expect(classifyColumn(column("created_by"))).toBe("user");
		expect(classifyColumn(column("user_sub"))).toBe("user");
		expect(classifyColumn(column("feedback_reporter"))).toBe("user");
	});

	test("leaves the kinds a declared type already settles", () => {
		expect(classifyColumn(column("owner", "Boolean"))).toBe("boolean");
		expect(classifyColumn(column("created_by", "Timestamp"))).toBe("temporal");
		expect(classifyColumn(column("owner_id", "Int64"))).toBe("number");
		expect(classifyColumn(column("owner", "Struct"))).toBe("json");
	});

	test("leaves text that only sounds like a person", () => {
		expect(classifyColumn(column("username"))).toBe("text");
		expect(classifyColumn(column("group_by"))).toBe("text");
		expect(classifyColumn(column("subject"))).toBe("text");
	});
});

describe("classifyResultColumn", () => {
	test("keeps the user kind when a row actually names an account", () => {
		const rows = [{ created_by: "system" }, { created_by: SUB }];
		expect(classifyResultColumn(column("created_by"), rows)).toBe("user");
	});

	test("demotes a person-named column that holds no account at all", () => {
		// The doc fixtures' `owner` column holds team names, not people.
		const rows = [{ owner: "Core Experience" }, { owner: "Trust" }];
		expect(classifyResultColumn(column("owner"), rows)).toBe("text");
	});

	test("trusts the name when there is nothing to sample", () => {
		expect(classifyResultColumn(column("created_by"), [])).toBe("user");
	});

	test("leaves every other kind exactly as classifyColumn had it", () => {
		const rows = [{ score: 1 }];
		expect(classifyResultColumn(column("score", "Float64"), rows)).toBe(
			"number",
		);
		expect(classifyResultColumn(column("username"), rows)).toBe("text");
	});
});
