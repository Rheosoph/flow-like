import { readFile } from "node:fs/promises";
import { FlowLikeClient } from "@flow-like/sdk";

const appId = process.env.APP_ID;
const eventId = process.env.EVENT_ID;
if (!appId || !eventId) throw new Error("Set APP_ID and EVENT_ID.");
const payload = JSON.parse(await readFile(new URL("./request.json", import.meta.url), "utf8"));
const client = new FlowLikeClient();
for await (const event of client.triggerEvent(appId, eventId, payload)) {
  console.log(event.data);
}
