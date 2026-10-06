import { afterAll, afterEach, describe, expect, test } from "bun:test";
import {
	byRole,
	click,
	installDom,
} from "../settings/devices/testing/dom-harness";

const dom = installDom();
const { ProviderModelReasoningPicker } = await import(
	"./provider-model-reasoning-picker"
);

afterEach(dom.cleanup);
afterAll(dom.restore);

function ignore() {}

describe("provider, model and reasoning picker", () => {
	test("marks the models that run on one of the user's devices", async () => {
		const { container } = await dom.render(
			<ProviderModelReasoningPicker
				provider="bits"
				providers={[{ id: "bits", label: "Profile" }]}
				models={[
					{ id: "qwen", label: "Qwen3 8B on gpu-box", deviceId: "gpu-box" },
					{ id: "gpt", label: "GPT-5" },
				]}
				selectedModelId="gpt"
				selectedEffort=""
				onProviderChange={ignore}
				onModelChange={ignore}
				onEffortChange={ignore}
			/>,
		);
		await click(byRole("button", /^Provider and model/, container));
		const marked = document.querySelectorAll("[data-device-model]");
		expect(marked).toHaveLength(1);
		expect(marked[0]?.closest("button")?.textContent).toBe(
			"Qwen3 8B on gpu-boxDevice",
		);
	});
});
