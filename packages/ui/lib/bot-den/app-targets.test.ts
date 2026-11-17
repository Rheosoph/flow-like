import { describe, expect, it } from "bun:test";
import { appDenProps } from "./app-targets";
import { DEN_TARGET_ATTRIBUTE, parseDenTarget } from "./targets";

describe("appDenProps", () => {
	it("describes a thing of an app", () => {
		const props = appDenProps({
			kind: "board",
			appId: "a1",
			id: "b1",
			label: " Intake ",
		});
		expect(parseDenTarget(props[DEN_TARGET_ATTRIBUTE])).toEqual({
			kind: "board",
			app_id: "a1",
			id: "b1",
			label: "Intake",
		});
	});

	it("carries the board of a board-scoped thing", () => {
		const props = appDenProps({
			kind: "variable",
			appId: "a1",
			boardId: "b1",
			id: "v1",
			label: "token",
		});
		expect(parseDenTarget(props[DEN_TARGET_ATTRIBUTE])).toEqual({
			kind: "variable",
			app_id: "a1",
			board_id: "b1",
			id: "v1",
			label: "token",
		});
	});

	it("tags nothing until the app, id and name are known", () => {
		expect(appDenProps({ kind: "app", appId: "a1", id: "a1" })).toEqual({});
		expect(
			appDenProps({ kind: "app", appId: null, id: "a1", label: "x" }),
		).toEqual({});
		expect(
			appDenProps({ kind: "app", appId: "a1", id: "", label: "x" }),
		).toEqual({});
	});
});
