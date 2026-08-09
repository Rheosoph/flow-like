#!/usr/bin/env node
import WebSocket from "ws";

// Create two connections - they should hit different PM2 workers
const ws1 = new WebSocket("ws://localhost:4444");
const ws2 = new WebSocket("ws://localhost:4444");

let ws1Ready = false;
let ws2Ready = false;
let receivedCount = 0;

ws1.on("open", () => {
	console.log("✓ Client 1 connected");
	ws1.send(JSON.stringify({ type: "subscribe", topics: ["fanout-test"] }));
	ws1Ready = true;
	checkAndPublish();
});

ws2.on("open", () => {
	console.log("✓ Client 2 connected");
	ws2.send(JSON.stringify({ type: "subscribe", topics: ["fanout-test"] }));
	ws2Ready = true;
	checkAndPublish();
});

function checkAndPublish() {
	if (ws1Ready && ws2Ready) {
		setTimeout(() => {
			console.log("\n→ Client 1 publishing message...");
			ws1.send(
				JSON.stringify({
					type: "publish",
					topic: "fanout-test",
					data: { test: "Redis fanout test", timestamp: Date.now() },
				}),
			);
		}, 100);
	}
}

ws1.on("message", (data) => {
	const msg = JSON.parse(data.toString());
	if (msg.type === "publish") {
		receivedCount++;
		console.log(`✓ Client 1 received message (clients: ${msg.clients})`);
		checkResults();
	}
});

ws2.on("message", (data) => {
	const msg = JSON.parse(data.toString());
	if (msg.type === "publish") {
		receivedCount++;
		console.log(`✓ Client 2 received message (clients: ${msg.clients})`);
		checkResults();
	}
});

function checkResults() {
	if (receivedCount >= 1) {
		setTimeout(() => {
			console.log("\n=== Results ===");
			console.log(`✓ Messages received: ${receivedCount}`);
			if (receivedCount >= 1) {
				console.log("✓ Redis pub/sub fanout is WORKING!");
			} else {
				console.log("✗ Redis fanout may not be working properly");
			}
			ws1.close();
			ws2.close();
			setTimeout(() => process.exit(0), 100);
		}, 300);
	}
}

ws1.on("error", (err) => console.error("✗ Client 1 error:", err.message));
ws2.on("error", (err) => console.error("✗ Client 2 error:", err.message));

setTimeout(() => {
	console.error("\n✗ Test timeout");
	process.exit(1);
}, 5000);
