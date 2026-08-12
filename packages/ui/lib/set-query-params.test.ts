import { describe, expect, test } from "bun:test";
import {
	type IQueryParamRequest,
	QUERY_PARAM_REQUEST_TTL_MS,
	nextQueryParamRequest,
} from "./set-query-params";

const use = (search: string) => ({ pathname: "/use", search });
const T0 = 1_000_000;

describe("query parameter requests", () => {
	test("sets a parameter on the live url when nothing is in flight", () => {
		expect(
			nextQueryParamRequest(use("?id=app"), null, "eventId", "e1", T0),
		).toEqual({
			pathname: "/use",
			from: "id=app",
			to: "id=app&eventId=e1",
			at: T0,
		});
	});

	test("does not navigate when the value already matches", () => {
		expect(
			nextQueryParamRequest(
				use("?id=app&eventId=e1"),
				null,
				"eventId",
				"e1",
				T0,
			),
		).toBeNull();
		expect(
			nextQueryParamRequest(use("?id=app"), null, "eventId", undefined, T0),
		).toBeNull();
	});

	test("does not navigate for a value that only differs by encoding", () => {
		expect(
			nextQueryParamRequest(use("?id=app&route=/"), null, "route", "/", T0),
		).toBeNull();
	});

	test("layers onto a request the browser has not committed yet", () => {
		const first = nextQueryParamRequest(
			use("?id=app"),
			null,
			"sessionId",
			"s1",
			T0,
		);
		const second = nextQueryParamRequest(
			use("?id=app"),
			first,
			"eventId",
			"e1",
			T0 + 5,
		);

		expect(second?.to).toBe("id=app&sessionId=s1&eventId=e1");
		expect(second?.from).toBe("id=app");
		expect(second?.at).toBe(T0);
	});

	test("drops a parameter without clobbering one that is still in flight", () => {
		const first = nextQueryParamRequest(
			use("?id=app&eventId=e1"),
			null,
			"sessionId",
			"s1",
			T0,
		);

		expect(
			nextQueryParamRequest(
				use("?id=app&eventId=e1"),
				first,
				"eventId",
				undefined,
				T0 + 5,
			)?.to,
		).toBe("id=app&sessionId=s1");
	});

	test("reads the live url again once the request committed", () => {
		const first = nextQueryParamRequest(
			use("?id=app"),
			null,
			"eventId",
			"e1",
			T0,
		);

		expect(
			nextQueryParamRequest(
				use("?id=app&eventId=e1"),
				first,
				"sessionId",
				"s1",
				T0 + 5,
			)?.to,
		).toBe("id=app&eventId=e1&sessionId=s1");
	});

	test("abandons a stale request when something else navigated", () => {
		const stale: IQueryParamRequest = {
			pathname: "/use",
			from: "id=app-a",
			to: "id=app-a&eventId=e1",
			at: T0,
		};

		expect(
			nextQueryParamRequest(use("?id=app-b"), stale, "sessionId", "s1", T0 + 5)
				?.to,
		).toBe("id=app-b&sessionId=s1");
	});

	test("ignores a request issued for another route", () => {
		const other: IQueryParamRequest = {
			pathname: "/chat",
			from: "id=app",
			to: "id=app&sessionId=s1",
			at: T0,
		};

		expect(
			nextQueryParamRequest(use("?id=app"), other, "eventId", "e1", T0 + 5)?.to,
		).toBe("id=app&eventId=e1");
	});

	test("forgets a request the user navigated away from and back to", () => {
		const abandoned: IQueryParamRequest = {
			pathname: "/use",
			from: "id=app",
			to: "id=app&eventId=e1",
			at: T0,
		};

		expect(
			nextQueryParamRequest(
				use("?id=app"),
				abandoned,
				"sessionId",
				"s1",
				T0 + QUERY_PARAM_REQUEST_TTL_MS,
			)?.to,
		).toBe("id=app&sessionId=s1");
	});
});
