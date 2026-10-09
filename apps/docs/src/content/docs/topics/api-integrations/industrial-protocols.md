---
title: Industrial protocols
description: Connect workflows to industrial devices, controllers, and message brokers
sidebar:
  order: 3
---

Industrial nodes connect a workflow to a device or broker, keep the connection for the current run, and pass incoming data to a referenced function. Standard desktop, server, and executor builds include the industrial catalog. An executor still needs network access, device permissions, and any controller driver required by the selected protocol.

## Choose a protocol and role

| Protocol | Available operations | Deployment requirements |
|---|---|---|
| Modbus TCP / RTU | Persistent client, read coils and registers, write coils and holding registers, poll | TCP endpoint or local serial adapter; configure unit ID and register layout |
| OPC UA | Client sessions, browse, scalar read/write, native monitored-item subscriptions | Server endpoint, security policy, trusted certificates, and local app storage for the PKI store |
| MQTT | Client publish/subscribe and embedded broker | TCP or TLS; binary payloads and Last Will supported; embedded broker accepts clean sessions and QoS 0/1 |
| Sparkplug B | Protobuf codec, birth/data/death state, shared sequence tracking, host STATE, rebirth detection | MQTT transport and persisted birth/death sequence |
| NATS / JetStream | Core publish/subscribe; persistent JetStream publishing and durable pull consumers | Existing NATS server; JetStream streams must already exist |
| Kafka / Redpanda | Publish, consume, commit after successful handling | Local executor with access to all advertised brokers |
| RabbitMQ / AMQP 0-9-1 | Publish with broker confirmations, consume with manual acknowledgements | Existing broker, exchange, and queue routing |
| Redis Streams | XADD, consumer-group reads, acknowledgements | Standalone Redis; use a stable consumer name to replay its pending entries |
| Zenoh | Publish, subscribe, get, and query replies | Local executor with access to configured or discovered peers |
| Iroh | Authenticated peer connections, listen, send, receive | Local executor; both peers must use Flow-Like's `flow-like/messages/2` ALPN and framing |
| ADS / TwinCAT | Read/write and native notifications | Configured ADS route and AMS Net IDs |
| EtherNet/IP / CIP | Explicit Logix tag read/write and polling | Compatible Rockwell Logix controller; implicit Class 1 cyclic I/O is not implemented |
| EtherCAT | Native master, topology inspection, cyclic process data, output updates, startup CoE SDO writes | Linux or macOS, a dedicated Ethernet interface, and raw-interface permissions |
| PROFIBUS | Native DP-V0 master, parameterization/configuration, cyclic process data, Clear/Operate/Stop | Local serial PHY adapter, station timings, and configuration derived from device GSD files |
| PROFINET and cifX fieldbuses | Native controller process-image reads/writes, bus state, polling | Installed Hilscher cifX controller, driver, protocol firmware, and commissioned bus configuration |
| IO-Link | Master discovery, process data, indexed ISDU access, polling | Master implementing the [IO-Link JSON Integration 2.0 REST API](https://github.com/iolinkcommunity/JSON_for_IO-Link/tree/2.0.0) |
| HART | Commands and read-only command polling over serial, transparent TCP, or HART-IP v1 Token Passing | Local HART serial modem or compatible network endpoint |

The cifX nodes call the installed controller's native driver. The card firmware runs the fieldbus. They do not provide a portable software PROFINET controller or configure an arbitrary vendor's stack. Linux uses the supported LinuxCIFXDrv 3.x ABI; Windows uses the installed system driver. Supply absolute driver and firmware paths. The driver and protocol firmware are not bundled.

The embedded MQTT broker keeps retained messages in memory for its current run. It does not provide durable sessions or a broker account/ACL system. Use an existing MQTT broker when the deployment needs those features. The MQTT client also supports QoS 2 when connected to a broker that implements it. MQTT acknowledgements confirm transport receipt, independently of workflow handler completion.

Iroh Send waits for the receiving peer to read and acknowledge the complete frame. The acknowledgement precedes workflow handler completion. Protocol version 2 uses bidirectional streams for this exchange and cannot connect to version 1 peers; update both ends together.

IO-Link electrical master control requires a hardware-specific stack and is not implemented by the REST adapter. OPC UA PubSub and DDS remain outside the current catalog. These distinctions matter when selecting hardware: a protocol name alone does not identify the controller role or supported transport.

## Connect once, then use the session

The industrial Connect nodes return a session reference. Pass that reference to read, write, publish, consume, or disconnect requests within the same workflow run. Saving its JSON does not preserve a connection across runs or executor restarts. Cancellation removes the cached session and signals active consumers; Disconnect performs the protocol's close operation.

For example, **Connect Modbus** accepts:

```json
{
  "transport": "tcp",
  "endpoint": "192.0.2.10:502",
  "timeout_ms": 3000
}
```

Use its result as `session` in **Read Modbus**:

```json
{
  "session": { "ref_id": "<connect result>", "protocol": "modbus" },
  "request": {
    "unit_id": 1,
    "kind": "holding_registers",
    "start_address": 0,
    "count": 2,
    "timeout_ms": 3000
  }
}
```

Register addresses are zero-based. Use **Decode Sensor Registers** when the device encodes a number across multiple registers; byte order and word order must match its register map.

## Run a listener from a daemon event

Start the workflow with a daemon event, connect, and enter a Subscribe, Consume, or Poll node. Reference exactly one handler function. Give that function a non-execution output named `event` to receive the full event object, or outputs named after individual event fields. The node's input and output schemas describe those fields.

The listener waits for each handler invocation to finish before processing the next delivery. JetStream, RabbitMQ, Redis Streams, and Kafka acknowledge or commit only after successful handling. Handler failure leaves the message unacknowledged. A crash between the handler's external side effect and the acknowledgement can still cause duplicate delivery, so use the broker message ID or your own business key when an operation must be idempotent.

Use consumer timeouts and bounded queues to match processing capacity. Core pub/sub messages are transient. Redis replays pending entries belonging to the configured consumer; it does not automatically claim abandoned entries from other consumers. Broker retention, queue bindings, stream creation, and recovery policy remain deployment configuration.

EtherCAT and PROFIBUS run their bus cycles on dedicated workers. Workflow handlers receive snapshots; slow workflow execution does not set the bus-cycle frequency. EtherCAT validates working counters and slave state. The executor does not promise hard real-time scheduling, and successful software tests do not establish timing or device interoperability for a physical installation.

EtherCAT Stop and failed startup request INIT and report cleanup failures. PROFIBUS Disconnect enters Clear and waits up to one second for a fresh data-exchange acknowledgement from every configured station. If stations do not acknowledge, Disconnect reports their addresses instead of claiming that Clear delivery completed.

## Publish Sparkplug B over MQTT

Sparkplug nodes build messages and state explicitly. The workflow owns publication order and durable state:

1. Load the previous `bd_seq`, then call **Prepare Sparkplug Session** with the group, edge-node ID, metric definitions, and timestamp. Persist the returned `state.bd_seq` before opening a new MQTT connection.
2. Put the returned `last_will` into MQTT connection configuration and set `clean_session` to `true`. Map numeric QoS `0`, `1`, or `2` to `AtMostOnce`, `AtLeastOnce`, or `ExactlyOnce` for MQTT nodes.
3. Connect MQTT, apply the `birth` action with **Sparkplug State Transition**, and publish each returned message in order. Feed its byte payload to MQTT's binary payload input. Use the returned state for the next transition.
4. Publish device births before their data. Node and device data share one sequence counter. Metric names, aliases, and datatypes must agree with the birth definitions.
5. Subscribe to commands and apply `birth` when a valid rebirth command is received. **Observe Sparkplug State** tracks an individual edge node and reports sequence gaps that require rebirth; it does not send the rebirth command itself.
6. For an orderly shutdown, publish the `death` transition before disconnecting MQTT. The broker sends the installed Last Will on an unexpected connection loss.

The protobuf codec uses protobuf JSON field names. Represent 64-bit integer values as decimal strings, for example `{"name":"count","datatype":8,"longValue":"123"}`. Birth and data transitions require each value field to match the birth datatype, including updates that omit `datatype`. For example, a Double metric uses `doubleValue`; array metrics use packed bytes in `bytesValue`. Host STATE messages use JSON on `spBv1.0/STATE/<host-id>` with `online` and `timestamp`, QoS 1, and retention.

## Executor access and builds

Serial ports, raw Ethernet, peer discovery, and native driver loading run on local or desktop executors. Kafka requires local execution because brokers can redirect clients to advertised addresses. In server execution, NATS, Redis, AMQP, and MQTT over TLS require IP-literal endpoints that pass the server's egress policy; TLS certificates must cover that address. MQTT over TLS currently accepts IPv4 literals and rejects IPv6 literals because of its SDK's hostname handling. Plain MQTT pins the approved resolved address. Local execution also supports TLS hostnames. HTTP-based IO-Link uses the guarded HTTP client.

The Rust adapters live in `flow-like-industrial`; node registration and execution integration live in `flow-like-catalog-industrial`. Neither requires the ML training engines. Product execution bundles enable them by default; metadata-only builds avoid linking the optional protocol SDKs. Existing inspection sensor node IDs are preserved, and `flow-like-ml-sensors` forwards its Rust API to the industrial crate. GenICam remains a separate `sensor-genicam` opt-in.
