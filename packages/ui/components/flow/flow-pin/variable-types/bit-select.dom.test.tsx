import { afterAll, afterEach, describe, expect, test } from "bun:test";
import type { MutableRefObject } from "react";
import type { IBit } from "../../../../lib/schema/bit/bit";
import { type IPin, IVariableType } from "../../../../lib/schema/flow/pin";
import { parseUint8ArrayToJson } from "../../../../lib/uint8";
import {
	allByRole,
	byRole,
	click,
	installDom,
} from "../../../settings/devices/testing/dom-harness";
import type { FlowSelectorData } from "../../flow-selector-data";

const dom = installDom();
const { BitVariable } = await import("./bit-select");
const { createEmptyFlowSelectorData, indexBitsByRef } = await import(
	"../../flow-selector-data"
);

afterEach(dom.cleanup);
afterAll(dom.restore);

function model(id: string, name: string, providerName: string): IBit {
	return {
		id,
		hub: "hub.test",
		type: "Llm",
		meta: { en: { name } },
		parameters: { provider: { provider_name: providerName } },
	} as unknown as IBit;
}

describe("bit select", () => {
	test("marks the models that run on one of the user's devices", async () => {
		const bits = [
			model("qwen-device", "Qwen3 8B on gpu-box", "device"),
			model("gpt", "GPT-5", "OpenAI"),
		];
		const data: FlowSelectorData = {
			...createEmptyFlowSelectorData(),
			bitOptions: bits,
			bitsByRef: indexBitsByRef(bits),
			bitsLoaded: true,
			loadBits: async () => bits,
		};
		const ref = { current: data } as MutableRefObject<FlowSelectorData>;
		const { container } = await dom.render(
			<BitVariable
				pin={{ friendly_name: "Model" } as IPin}
				value={undefined}
				setValue={() => undefined}
				selectorDataRef={ref}
			/>,
		);
		await click(byRole("combobox", undefined, container));
		const options = allByRole("option");
		expect(options.map((option) => option.textContent)).toEqual([
			"Qwen3 8B on gpu-boxDevice",
			"GPT-5",
		]);
		expect(document.querySelectorAll("[data-device-model]")).toHaveLength(1);
	});
});

test("Load Bit can select decision models and stores only their reference", async () => {
	const decision = {
		...model("decisions", "Decision model", "Local"),
		type: "SystemOne",
		parameters: {
			provider: {
				provider_name: "custom:systemone",
				params: { api_key: "private-key" },
			},
		},
	} as IBit;
	const bits = [model("chat", "Chat model", "Hosted"), decision];
	const data = {
		...createEmptyFlowSelectorData(),
		bitOptions: bits,
		bitsByRef: indexBitsByRef(bits),
		bitsLoaded: true,
		loadBits: async () => bits,
	};
	let selected: unknown;
	const { container } = await dom.render(
		<BitVariable
			pin={{ friendly_name: "Bit", data_type: IVariableType.String } as IPin}
			value={undefined}
			setValue={(value) => {
				selected = value;
			}}
			selectorDataRef={{ current: data }}
		/>,
	);
	await click(byRole("combobox", undefined, container));
	expect(allByRole("option").map((option) => option.textContent)).toEqual([
		"Chat model",
		"Decision model",
	]);
	await click(byRole("option", "Decision model"));
	expect(parseUint8ArrayToJson(selected as number[])).toBe(
		"hub.test:decisions",
	);
	expect(
		JSON.stringify(parseUint8ArrayToJson(selected as number[])),
	).not.toContain("private-key");
});
