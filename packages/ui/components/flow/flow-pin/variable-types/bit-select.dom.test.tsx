import { afterAll, afterEach, describe, expect, test } from "bun:test";
import type { MutableRefObject } from "react";
import type { IBit } from "../../../../lib/schema/bit/bit";
import type { IPin } from "../../../../lib/schema/flow/pin";
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
