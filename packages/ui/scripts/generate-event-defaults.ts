import { writeFile } from "node:fs/promises";
import { buildNativeEventDefaults } from "../lib/flowpilot/native-event-defaults";

await writeFile(
	new URL("../lib/flowpilot/generated/event-defaults.json", import.meta.url),
	`${JSON.stringify(buildNativeEventDefaults(), null, 2)}\n`,
);
