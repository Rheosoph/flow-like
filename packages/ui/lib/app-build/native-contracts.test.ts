import { describe, expect, it } from "vitest";
import { z } from "zod";
import contracts from "../flowpilot/generated/app-build-contracts.json";
import { instantiateAppCapability, listAppCapabilities } from "./capabilities";
import {
	approvalParametersSchema,
	crudParametersSchema,
} from "./capability-contract";
import { compileAppSpec } from "./compiler";
import { appBuildContractGuide, appSpecSchema } from "./contract";
import { INTERACTIVE_APP_BUILD_CAPABILITIES } from "./interactive-scenario-adapter";
import { appBuildStateSchema } from "./state-contract";

describe("native app-build contracts", () => {
	it("ships the current web contracts and capability definitions", () => {
		expect(contracts.guide).toEqual(appBuildContractGuide());
		expect(contracts.host_capabilities).toEqual(
			INTERACTIVE_APP_BUILD_CAPABILITIES,
		);
		expect(contracts.capabilities).toEqual(listAppCapabilities());
		expect(contracts.spec_schema).toEqual(
			z.toJSONSchema(appSpecSchema, { io: "input" }),
		);
		expect(contracts.state_schema).toEqual(
			z.toJSONSchema(appBuildStateSchema, { io: "input" }),
		);
		expect(contracts.parameter_schemas["crud.foundation"]).toEqual(
			z.toJSONSchema(crudParametersSchema, { io: "input" }),
		);
		expect(contracts.parameter_schemas["approval.foundation"]).toEqual(
			z.toJSONSchema(approvalParametersSchema, { io: "input" }),
		);
	});
	it("keeps native compiler and recipe fixtures tied to web behavior", () => {
		for (const vector of contracts.compiler_vectors) {
			expect(compileAppSpec(vector.input, vector)).toEqual(vector.output);
		}
		for (const input of contracts.invalid_vectors) {
			expect(() =>
				compileAppSpec(input, { app_id: "app", build_id: "build" }),
			).toThrow();
		}
		for (const vector of contracts.recipe_vectors) {
			expect(
				instantiateAppCapability(vector.id, vector.version, vector.parameters),
			).toEqual(vector.output);
		}
	});
});
