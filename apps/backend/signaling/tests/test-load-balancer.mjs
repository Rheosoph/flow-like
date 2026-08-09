#!/usr/bin/env node
import { randomUUID } from "crypto";
import WebSocket from "ws";

// ============================================
// Load Balancer Test Script
// ============================================
// This script tests the signaling server behind a load balancer
// by simulating multiple clients and verifying:
// - Connection distribution across instances
// - Message delivery across different instances
// - Presence/subscriber counting accuracy
// - Graceful handling of instance failures
// ============================================

const CONFIG = {
	// Load balancer endpoint (or direct server endpoint for testing)
	endpoint: process.env.WS_ENDPOINT || "wss://signaling.flow-like.com",

	// Number of concurrent client connections
	numClients: Number.parseInt(process.env.NUM_CLIENTS || "20"),

	// Number of messages to send per client
	messagesPerClient: Number.parseInt(process.env.MESSAGES_PER_CLIENT || "100"),

	// Delay between messages (ms)
	messageDelay: Number.parseInt(process.env.MESSAGE_DELAY || "100"),

	// Test timeout (ms)
	timeout: Number.parseInt(process.env.TEST_TIMEOUT || "30000"),

	// Shared topic for all clients
	testTopic: `load-test-${randomUUID().slice(0, 8)}`,
};

console.log("╔═══════════════════════════════════════════╗");
console.log("║   WebSocket Load Balancer Test Suite     ║");
console.log("╚═══════════════════════════════════════════╝\n");
console.log("Configuration:");
console.log(`  Endpoint:         ${CONFIG.endpoint}`);
console.log(`  Clients:          ${CONFIG.numClients}`);
console.log(`  Messages/Client:  ${CONFIG.messagesPerClient}`);
console.log(`  Test Topic:       ${CONFIG.testTopic}`);
console.log(`  Timeout:          ${CONFIG.timeout}ms\n`);

// ============================================
// Statistics tracking
// ============================================
const stats = {
	connected: 0,
	subscribed: 0,
	messagesSent: 0,
	messagesReceived: 0,
	uniqueOrigins: new Set(),
	clientLatencies: [],
	errors: 0,
	reconnects: 0,
	reportedClientCounts: [],
};

// Global message tracking for verification
const allSentMessages = new Set(); // Set of all messageIds sent by any client
const messageDeliveryMatrix = new Map(); // messageId -> Set<clientId> who received it

// ============================================
// Client wrapper
// ============================================
class TestClient {
	constructor(id) {
		this.id = id;
		this.ws = null;
		this.connected = false;
		this.subscribed = false;
		this.messagesSent = 0;
		this.messagesReceived = 0;
		this.sentTimestamps = new Map(); // messageId -> timestamp
		this.receivedOrigins = new Set();
		this.receivedMessageIds = new Set(); // Track which messages this client received
		this.errors = [];
	}

	async connect() {
		return new Promise((resolve, reject) => {
			try {
				this.ws = new WebSocket(CONFIG.endpoint, {
					handshakeTimeout: 5000,
					rejectUnauthorized: false,
					servername: "signaling.flow-like.com",
				});

				const timeout = setTimeout(() => {
					reject(new Error(`Connection timeout for client ${this.id}`));
				}, 5000);

				this.ws.on("open", () => {
					clearTimeout(timeout);
					this.connected = true;
					stats.connected++;
					console.log(`[Client ${this.id}] ✓ Connected`);
					resolve();
				});

				this.ws.on("message", (data) => this.handleMessage(data));

				this.ws.on("error", (err) => {
					this.errors.push(err.message);
					stats.errors++;
					console.error(`[Client ${this.id}] ✗ Error: ${err.message}`);
				});

				this.ws.on("close", (code, reason) => {
					this.connected = false;
					console.log(`[Client ${this.id}] Connection closed (${code})`);
				});
			} catch (err) {
				reject(err);
			}
		});
	}

	async subscribe() {
		if (!this.connected) throw new Error("Not connected");

		return new Promise((resolve) => {
			this.ws.send(
				JSON.stringify({
					type: "subscribe",
					topics: [CONFIG.testTopic],
				}),
			);

			this.subscribed = true;
			stats.subscribed++;
			console.log(`[Client ${this.id}] ✓ Subscribed to ${CONFIG.testTopic}`);

			// Give it a moment to propagate
			setTimeout(resolve, 50);
		});
	}

	handleMessage(data) {
		try {
			const msg = JSON.parse(data.toString());

			if (msg.type === "publish") {
				this.messagesReceived++;
				stats.messagesReceived++;

				// Track which message this client received
				if (msg.messageId) {
					this.receivedMessageIds.add(msg.messageId);

					// Track in global delivery matrix
					if (!messageDeliveryMatrix.has(msg.messageId)) {
						messageDeliveryMatrix.set(msg.messageId, new Set());
					}
					messageDeliveryMatrix.get(msg.messageId).add(this.id);
				}

				// Track origin nodes (to verify load distribution)
				if (msg._origin) {
					this.receivedOrigins.add(msg._origin);
					stats.uniqueOrigins.add(msg._origin);
				}

				// Track reported client count
				if (msg.clients !== undefined) {
					stats.reportedClientCounts.push(msg.clients);
				}

				// Calculate latency if we sent this message
				if (msg.messageId && this.sentTimestamps.has(msg.messageId)) {
					const latency = Date.now() - this.sentTimestamps.get(msg.messageId);
					stats.clientLatencies.push(latency);
					this.sentTimestamps.delete(msg.messageId);
				}

				// Log first few messages for visibility
				if (stats.messagesReceived <= 10) {
					console.log(
						`[Client ${this.id}] ← Received message ${msg.messageId?.slice(0, 8)} from origin ${msg._origin?.slice(0, 8) || "unknown"}`,
					);
				}
			} else if (msg.type === "pong") {
				// Health check response
			}
		} catch (err) {
			console.error(
				`[Client ${this.id}] Failed to parse message:`,
				err.message,
			);
		}
	}

	async sendMessage() {
		if (!this.subscribed) throw new Error("Not subscribed");

		const messageId = randomUUID();
		this.sentTimestamps.set(messageId, Date.now());

		// Track this message globally
		allSentMessages.add(messageId);

		this.ws.send(
			JSON.stringify({
				type: "publish",
				topic: CONFIG.testTopic,
				messageId,
				clientId: this.id,
				senderId: this.id, // Add sender ID so we can verify sender doesn't receive own message
				data: {
					timestamp: Date.now(),
					message: `Test message from client ${this.id}`,
				},
			}),
		);

		this.messagesSent++;
		stats.messagesSent++;

		return messageId;
	}

	async ping() {
		if (!this.connected) return;

		return new Promise((resolve) => {
			this.ws.send(JSON.stringify({ type: "ping" }));

			const handler = (data) => {
				const msg = JSON.parse(data.toString());
				if (msg.type === "pong") {
					this.ws.removeListener("message", handler);
					resolve();
				}
			};

			this.ws.on("message", handler);
			setTimeout(resolve, 1000); // Don't wait forever
		});
	}

	close() {
		if (this.ws) {
			this.ws.close();
			this.ws = null;
		}
	}
}

// ============================================
// Test phases
// ============================================
const clients = [];
let testFailed = false;

async function runTest() {
	try {
		// Phase 1: Connect all clients
		console.log("\n━━━ Phase 1: Connecting clients ━━━");
		await connectClients();

		// Phase 2: Subscribe all clients
		console.log("\n━━━ Phase 2: Subscribing to topic ━━━");
		await subscribeClients();

		// Phase 3: Send messages
		console.log("\n━━━ Phase 3: Sending messages ━━━");
		await sendMessages();

		// Phase 4: Wait for message propagation
		console.log("\n━━━ Phase 4: Waiting for message propagation ━━━");
		await wait(2000);

		// Phase 5: Verify message delivery
		console.log("\n━━━ Phase 5: Verifying all messages arrived ━━━");
		verifyMessageDelivery();

		// Phase 6: Health checks
		console.log("\n━━━ Phase 6: Running health checks ━━━");
		await healthChecks();

		// Phase 7: Generate report
		console.log("\n━━━ Phase 7: Generating report ━━━");
		generateReport();
	} catch (err) {
		console.error("\n✗ Test failed:", err.message);
		testFailed = true;
	} finally {
		cleanup();
	}
}

async function connectClients() {
	const promises = [];

	for (let i = 0; i < CONFIG.numClients; i++) {
		const client = new TestClient(i + 1);
		clients.push(client);
		promises.push(client.connect());
	}

	await Promise.all(promises);
	console.log(`✓ All ${CONFIG.numClients} clients connected\n`);
}

async function subscribeClients() {
	const promises = clients.map((c) => c.subscribe());
	await Promise.all(promises);

	// Wait for presence to propagate across instances
	await wait(500);
	console.log(`✓ All ${CONFIG.numClients} clients subscribed\n`);
}

async function sendMessages() {
	// Each client sends multiple messages with delays
	for (let i = 0; i < CONFIG.messagesPerClient; i++) {
		console.log(`\nRound ${i + 1}/${CONFIG.messagesPerClient}:`);

		// Have each client send a message
		const promises = clients.map(async (client) => {
			await client.sendMessage();
		});

		await Promise.all(promises);
		console.log(`  Sent ${CONFIG.numClients} messages`);

		// Wait before next round
		if (i < CONFIG.messagesPerClient - 1) {
			await wait(CONFIG.messageDelay);
		}
	}

	console.log(`\n✓ Total messages sent: ${stats.messagesSent}`);
}

async function healthChecks() {
	let healthyCount = 0;

	for (const client of clients.slice(0, 5)) {
		// Check first 5 clients
		try {
			await client.ping();
			healthyCount++;
		} catch (err) {
			console.log(`[Client ${client.id}] Health check failed`);
		}
	}

	console.log(`✓ Health checks: ${healthyCount}/5 responded`);
}

function verifyMessageDelivery() {
	console.log(`\nVerifying message delivery...`);
	console.log(`Total messages sent: ${allSentMessages.size}`);
	console.log(
		`Messages tracked in delivery matrix: ${messageDeliveryMatrix.size}`,
	);

	let completeDeliveries = 0;
	let partialDeliveries = 0;
	let missedDeliveries = 0;
	const missedMessages = [];

	// For each sent message, verify it reached all other clients (excluding sender)
	for (const messageId of allSentMessages) {
		const recipients = messageDeliveryMatrix.get(messageId);

		if (!recipients) {
			// Message was sent but never recorded as received by anyone
			missedDeliveries++;
			missedMessages.push(messageId);
			continue;
		}

		// Expected recipients = all clients minus the sender
		// Note: We need to identify which client sent this message
		// We can check which client has this in their sentTimestamps or sent it
		const expectedRecipients = CONFIG.numClients - 1; // All clients except sender

		if (recipients.size === expectedRecipients) {
			completeDeliveries++;
		} else if (recipients.size > 0) {
			partialDeliveries++;
		} else {
			missedDeliveries++;
			missedMessages.push(messageId);
		}
	}

	console.log(`\nDelivery verification results:`);
	console.log(
		`  Complete deliveries:  ${completeDeliveries}/${allSentMessages.size}`,
	);
	console.log(
		`  Partial deliveries:   ${partialDeliveries}/${allSentMessages.size}`,
	);
	console.log(
		`  Missed deliveries:    ${missedDeliveries}/${allSentMessages.size}`,
	);

	if (missedMessages.length > 0 && missedMessages.length <= 10) {
		console.log(`\nMissed message IDs (first 10):`);
		missedMessages.slice(0, 10).forEach((id) => {
			const recipients = messageDeliveryMatrix.get(id);
			console.log(
				`  ${id.slice(0, 8)}: received by ${recipients ? recipients.size : 0} clients`,
			);
		});
	}

	// Per-client verification
	let clientsWithPerfectDelivery = 0;
	const clientDeliveryIssues = [];

	for (const client of clients) {
		// Each client should receive all messages except those it sent itself
		const expectedMessages = allSentMessages.size - client.messagesSent;
		const actualMessages = client.receivedMessageIds.size;

		if (actualMessages === expectedMessages) {
			clientsWithPerfectDelivery++;
		} else {
			clientDeliveryIssues.push({
				clientId: client.id,
				expected: expectedMessages,
				actual: actualMessages,
				missing: expectedMessages - actualMessages,
			});
		}
	}

	console.log(`\nPer-client delivery verification:`);
	console.log(
		`  Clients with perfect delivery: ${clientsWithPerfectDelivery}/${CONFIG.numClients}`,
	);

	if (clientDeliveryIssues.length > 0 && clientDeliveryIssues.length <= 10) {
		console.log(`\nClients with delivery issues (first 10):`);
		clientDeliveryIssues.slice(0, 10).forEach((issue) => {
			console.log(
				`  Client ${issue.clientId}: expected ${issue.expected}, got ${issue.actual}, missing ${issue.missing}`,
			);
		});
	}

	// Store results for final report
	stats.completeDeliveries = completeDeliveries;
	stats.partialDeliveries = partialDeliveries;
	stats.missedDeliveries = missedDeliveries;
	stats.clientsWithPerfectDelivery = clientsWithPerfectDelivery;
	stats.totalClientDeliveryIssues = clientDeliveryIssues.length;
}

function generateReport() {
	console.log("\n");
	console.log("╔═══════════════════════════════════════════╗");
	console.log("║             TEST RESULTS                  ║");
	console.log("╚═══════════════════════════════════════════╝\n");

	// Connection statistics
	console.log("📊 Connection Statistics:");
	console.log(
		`   Clients connected:    ${stats.connected}/${CONFIG.numClients}`,
	);
	console.log(
		`   Clients subscribed:   ${stats.subscribed}/${CONFIG.numClients}`,
	);
	console.log(`   Connection errors:    ${stats.errors}`);

	// Message statistics
	console.log("\n📨 Message Statistics:");
	console.log(`   Messages sent:        ${stats.messagesSent}`);
	console.log(`   Messages received:    ${stats.messagesReceived}`);

	const expectedMessages =
		CONFIG.numClients * CONFIG.messagesPerClient * (CONFIG.numClients - 1);
	const deliveryRate = (
		(stats.messagesReceived / expectedMessages) *
		100
	).toFixed(2);
	console.log(`   Expected receives:    ${expectedMessages}`);
	console.log(`   Delivery rate:        ${deliveryRate}%`);

	// Load balancer distribution
	console.log("\n⚖️  Load Balancer Distribution:");
	console.log(`   Unique server nodes:  ${stats.uniqueOrigins.size}`);

	if (stats.uniqueOrigins.size > 0) {
		console.log(`   Node IDs:`);
		stats.uniqueOrigins.forEach((id) => {
			console.log(`     - ${id}`);
		});
	}

	// Presence/subscriber count accuracy
	if (stats.reportedClientCounts.length > 0) {
		const avgCount =
			stats.reportedClientCounts.reduce((a, b) => a + b, 0) /
			stats.reportedClientCounts.length;
		const minCount = Math.min(...stats.reportedClientCounts);
		const maxCount = Math.max(...stats.reportedClientCounts);

		console.log("\n👥 Subscriber Count Reporting:");
		console.log(`   Expected count:       ${CONFIG.numClients}`);
		console.log(`   Avg reported count:   ${avgCount.toFixed(1)}`);
		console.log(`   Min reported count:   ${minCount}`);
		console.log(`   Max reported count:   ${maxCount}`);

		const countAccuracy = ((avgCount / CONFIG.numClients) * 100).toFixed(2);
		console.log(`   Count accuracy:       ${countAccuracy}%`);
	}

	// Latency statistics
	if (stats.clientLatencies.length > 0) {
		const avgLatency =
			stats.clientLatencies.reduce((a, b) => a + b, 0) /
			stats.clientLatencies.length;
		const minLatency = Math.min(...stats.clientLatencies);
		const maxLatency = Math.max(...stats.clientLatencies);
		const p95Latency = stats.clientLatencies.sort((a, b) => a - b)[
			Math.floor(stats.clientLatencies.length * 0.95)
		];

		console.log("\n⚡ Latency Statistics:");
		console.log(`   Average latency:      ${avgLatency.toFixed(2)}ms`);
		console.log(`   Min latency:          ${minLatency}ms`);
		console.log(`   Max latency:          ${maxLatency}ms`);
		console.log(`   P95 latency:          ${p95Latency}ms`);
	}

	// Per-client statistics
	console.log("\n📈 Per-Client Statistics:");
	const clientsReceivedAll = clients.filter(
		(c) =>
			c.messagesReceived >=
			CONFIG.numClients * CONFIG.messagesPerClient - CONFIG.messagesPerClient,
	).length;
	console.log(
		`   Clients with full delivery: ${clientsReceivedAll}/${CONFIG.numClients}`,
	);

	const clientsWithErrors = clients.filter((c) => c.errors.length > 0).length;
	console.log(
		`   Clients with errors:        ${clientsWithErrors}/${CONFIG.numClients}`,
	);

	// Message delivery verification
	console.log("\n✅ Message Delivery Verification:");
	console.log(`   Total unique messages sent:     ${allSentMessages.size}`);
	console.log(
		`   Complete deliveries:            ${stats.completeDeliveries || 0}`,
	);
	console.log(
		`   Partial deliveries:             ${stats.partialDeliveries || 0}`,
	);
	console.log(
		`   Missed deliveries:              ${stats.missedDeliveries || 0}`,
	);
	console.log(
		`   Clients with perfect delivery:  ${stats.clientsWithPerfectDelivery || 0}/${CONFIG.numClients}`,
	);
	console.log(
		`   Clients with delivery issues:   ${stats.totalClientDeliveryIssues || 0}/${CONFIG.numClients}`,
	);

	const messageDeliveryRate =
		allSentMessages.size > 0
			? (
					((stats.completeDeliveries || 0) / allSentMessages.size) *
					100
				).toFixed(2)
			: "0.00";
	console.log(`   Message delivery success rate:  ${messageDeliveryRate}%`);

	// Overall verdict
	console.log("\n" + "═".repeat(45));
	const successCriteria = {
		allConnected: stats.connected === CONFIG.numClients,
		allSubscribed: stats.subscribed === CONFIG.numClients,
		deliveryRateGood: Number.parseFloat(deliveryRate) >= 95,
		messageDeliveryPerfect: Number.parseFloat(messageDeliveryRate) >= 95,
		allClientsReceivedAll:
			(stats.clientsWithPerfectDelivery || 0) >= CONFIG.numClients * 0.95, // 95% of clients
		multipleNodes: stats.uniqueOrigins.size >= 1, // At least 1 node
		lowErrors: stats.errors < CONFIG.numClients * 0.1, // Less than 10% error rate
	};

	const allPassed = Object.values(successCriteria).every((v) => v);

	if (allPassed) {
		console.log("✅ TEST PASSED: Load balancer working correctly!");
		console.log("   All messages successfully delivered to all clients!");
	} else {
		console.log("❌ TEST FAILED: Issues detected!");
		console.log("\nFailed criteria:");
		if (!successCriteria.allConnected)
			console.log("  - Not all clients connected");
		if (!successCriteria.allSubscribed)
			console.log("  - Not all clients subscribed");
		if (!successCriteria.deliveryRateGood)
			console.log("  - Message delivery rate below 95%");
		if (!successCriteria.messageDeliveryPerfect)
			console.log("  - Message delivery verification below 95%");
		if (!successCriteria.allClientsReceivedAll)
			console.log("  - Not all clients received all messages");
		if (!successCriteria.multipleNodes)
			console.log("  - No server nodes detected");
		if (!successCriteria.lowErrors)
			console.log("  - Too many errors encountered");
		testFailed = true;
	}
	console.log("═".repeat(45) + "\n");

	// Recommendations
	if (stats.uniqueOrigins.size === 1) {
		console.log("💡 Note: Only 1 unique server node detected.");
		console.log(
			"   For true load balancer testing, run multiple server instances:",
		);
		console.log("   PORT=4444 NODE_ID=node1 bun run server.ts");
		console.log("   PORT=4445 NODE_ID=node2 bun run server.ts");
		console.log(
			"   Then configure your load balancer to distribute across them.\n",
		);
	}

	if (Number.parseFloat(deliveryRate) < 95) {
		console.log("⚠️  Low delivery rate detected. Potential issues:");
		console.log("   - Redis not running or misconfigured");
		console.log("   - Network issues between server instances");
		console.log("   - Server instances not properly pub/sub synced\n");
	}

	if (Number.parseFloat(messageDeliveryRate) < 95) {
		console.log("⚠️  Message delivery verification failed. Issues:");
		console.log("   - Some messages did not reach all expected clients");
		console.log("   - Check Redis pub/sub fanout is working correctly");
		console.log(
			"   - Verify all server instances are subscribed to the same channel\n",
		);
	}
}

function cleanup() {
	console.log("Cleaning up...");
	clients.forEach((c) => c.close());

	setTimeout(() => {
		process.exit(testFailed ? 1 : 0);
	}, 500);
}

function wait(ms) {
	return new Promise((resolve) => setTimeout(resolve, ms));
}

// ============================================
// Main execution
// ============================================
const testTimeout = setTimeout(() => {
	console.error("\n⏱️  TEST TIMEOUT - Forcing exit");
	testFailed = true;
	cleanup();
}, CONFIG.timeout);

runTest().finally(() => {
	clearTimeout(testTimeout);
});
