import { describe, expect, test } from "bun:test";
import {
	HOME_DATA_EMPTY_LABEL,
	homeDataFieldLabel,
	homeDataInlineFieldLabel,
	homeDataText,
} from "./home-data-text";

describe("homeDataText", () => {
	test("reads missing values as the empty label", () => {
		expect(homeDataText(null)).toBe(HOME_DATA_EMPTY_LABEL);
		expect(homeDataText(undefined)).toBe("No value");
	});

	test("keeps falsy values that are real values", () => {
		expect(homeDataText(0)).toBe("0");
		expect(homeDataText(false)).toBe("false");
		expect(homeDataText("")).toBe("");
	});

	test("serializes objects instead of printing [object Object]", () => {
		expect(homeDataText({ plan: "pro", seats: 3 })).toBe(
			'{"plan":"pro","seats":3}',
		);
		expect(homeDataText(["a", "b"])).toBe('["a","b"]');
	});
});

describe("homeDataFieldLabel", () => {
	test("spaces snake_case, kebab-case and dotted names", () => {
		expect(homeDataFieldLabel("relevance_score")).toBe("Relevance score");
		expect(homeDataFieldLabel("last_updated")).toBe("Last updated");
		expect(homeDataFieldLabel("order-total.net")).toBe("Order total net");
		expect(homeDataFieldLabel("address_line_2")).toBe("Address line 2");
	});

	test("splits camelCase and PascalCase", () => {
		expect(homeDataFieldLabel("feedsChecked")).toBe("Feeds checked");
		expect(homeDataFieldLabel("LastSuccessAt")).toBe("Last success at");
		expect(homeDataFieldLabel("p95LatencyMs")).toBe("P95 latency ms");
	});

	test("reads SCREAMING_CASE as ordinary words", () => {
		expect(homeDataFieldLabel("TOTAL_AMOUNT")).toBe("Total amount");
		expect(homeDataFieldLabel("CREATED_AT", "temporal")).toBe("Created");
	});

	test("keeps a lone all-caps word as the acronym it is", () => {
		expect(homeDataFieldLabel("MRR")).toBe("MRR");
		expect(homeDataFieldLabel("NPS", "number")).toBe("NPS");
	});

	test("keeps acronyms upper case wherever they sit", () => {
		expect(homeDataFieldLabel("api_url")).toBe("API URL");
		expect(homeDataFieldLabel("URL_PATH")).toBe("URL path");
		expect(homeDataFieldLabel("owner_id")).toBe("Owner ID");
		expect(homeDataFieldLabel("ipAddress")).toBe("IP address");
		expect(homeDataFieldLabel("userID")).toBe("User ID");
	});

	test("keeps an upper-case run in a mixed-case name as its own word", () => {
		expect(homeDataFieldLabel("total_EUR")).toBe("Total EUR");
		expect(homeDataFieldLabel("HTTPStatus")).toBe("HTTP status");
		expect(homeDataFieldLabel("parseXMLBody")).toBe("Parse XML body");
	});

	test("keeps a plural acronym in one piece", () => {
		expect(homeDataFieldLabel("userIDs")).toBe("User IDs");
		expect(homeDataFieldLabel("imageURLsCount")).toBe("Image URLs count");
		expect(homeDataFieldLabel("user_ids")).toBe("User IDs");
		expect(homeDataFieldLabel("API_URLS")).toBe("API URLs");
		expect(homeDataFieldLabel("HTTPSession")).toBe("HTTP session");
		expect(homeDataFieldLabel("status")).toBe("Status");
	});

	test("drops a suffix only when the kind already implies it", () => {
		expect(homeDataFieldLabel("first_seen_at", "temporal")).toBe("First seen");
		expect(homeDataFieldLabel("firstSeenAt", "temporal")).toBe("First seen");
		expect(homeDataFieldLabel("updated_on", "temporal")).toBe("Updated");
		expect(homeDataFieldLabel("owner_id", "user")).toBe("Owner");
		expect(homeDataFieldLabel("user_sub", "user")).toBe("User");

		expect(homeDataFieldLabel("first_seen_at")).toBe("First seen at");
		expect(homeDataFieldLabel("first_seen_at", "number")).toBe("First seen at");
		expect(homeDataFieldLabel("owner_id", "temporal")).toBe("Owner ID");
		expect(homeDataFieldLabel("created_at", "user")).toBe("Created at");
	});

	test("never drops the last remaining word", () => {
		expect(homeDataFieldLabel("at", "temporal")).toBe("At");
		expect(homeDataFieldLabel("id", "user")).toBe("ID");
		expect(homeDataFieldLabel("user_id_ref", "user")).toBe("User");
	});

	test("keeps letters outside ASCII inside their words", () => {
		expect(homeDataFieldLabel("größe_kg")).toBe("Größe kg");
		expect(homeDataFieldLabel("änderungsDatum")).toBe("Änderungs datum");
		expect(homeDataFieldLabel("prix_été")).toBe("Prix été");
	});

	test("falls back to the raw name when nothing is left to label", () => {
		expect(homeDataFieldLabel("__")).toBe("__");
		expect(homeDataFieldLabel("")).toBe("");
	});
});

describe("homeDataInlineFieldLabel", () => {
	test("lowercases the first letter for use mid-sentence", () => {
		expect(homeDataInlineFieldLabel("first_seen_at", "temporal")).toBe(
			"first seen",
		);
		expect(homeDataInlineFieldLabel("net_amount")).toBe("net amount");
		expect(homeDataInlineFieldLabel("größe_kg")).toBe("größe kg");
	});

	test("leaves a leading acronym as written", () => {
		expect(homeDataInlineFieldLabel("url_path")).toBe("URL path");
		expect(homeDataInlineFieldLabel("api_url")).toBe("API URL");
		expect(homeDataInlineFieldLabel("id")).toBe("ID");
		expect(homeDataInlineFieldLabel("user_ids")).toBe("user IDs");
		expect(homeDataInlineFieldLabel("ids_seen")).toBe("IDs seen");
	});

	test("leaves a leading upper-case code with digits as written", () => {
		expect(homeDataInlineFieldLabel("Q3_revenue")).toBe("Q3 revenue");
		expect(homeDataInlineFieldLabel("s3_path")).toBe("S3 path");
		expect(homeDataInlineFieldLabel("p95LatencyMs")).toBe("P95 latency ms");
		expect(homeDataInlineFieldLabel("status_2")).toBe("status 2");
	});

	test("lowercases a one-letter label", () => {
		expect(homeDataInlineFieldLabel("x")).toBe("x");
		expect(homeDataInlineFieldLabel("Y")).toBe("y");
	});
});
