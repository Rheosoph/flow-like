// industrial — FlowScript node declarations (generated, do not edit).
// One `function` per catalog node, grouped by FlowScript namespace. Call a node as
// `ns::alias({ pin: value })`, or write `use ns::*` once at the top of a .flow file and
// call `alias({ pin: value })`. Leading inputs may also be passed positionally, in
// declaration order, before the named object: `ns::alias(a, b, { pin: value })`. A
// `this: T` parameter marks the receiver pin: such a node is also a method on that value
// (`x.alias(...)`, remaining inputs positional or named).
// JSDoc tags carry the node type (`@node`), the receiver pin (`@receiver`) and the legacy
// camelCase spelling (`@alias`), which is still accepted.

declare namespace ads {
    // === Industrial/ADS ===

    /**
     * Open a persistent ADS session. The PLC must have a route for the source AMS address.
     * @node industrial_ads_connect @alias industrialAdsConnect
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function connect({ request: Struct }): Struct;

    /**
     * Stop ADS notification listeners and close the connection
     * @node industrial_ads_disconnect @alias industrialAdsDisconnect
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function disconnect({ request: Struct }): bool;

    /**
     * Read bytes from a named PLC symbol or ADS index address
     * @node industrial_ads_read @alias industrialAdsRead
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function read({ request: Struct }): int[];

    /**
     * Call the handler with data and the PLC timestamp for each server notification. Stops on handler error or buffer overflow.
     * @node industrial_ads_subscribe @alias industrialAdsSubscribe
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function subscribe({ request: Struct }): int;

    /**
     * Write bytes to a named PLC symbol or ADS index address
     * @node industrial_ads_write @alias industrialAdsWrite
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function write({ request: Struct }): bool;
}

declare namespace amqp {
    // === Industrial/RabbitMQ ===

    /**
     * Open a RabbitMQ session with bounded connection timeout
     * @node industrial_amqp_connect @alias industrialAmqpConnect
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function connect({ request: Struct }): Struct;

    /**
     * Run a referenced handler for each message and acknowledge durable deliveries only after successful processing
     * @node industrial_amqp_consume @alias industrialAmqpConsume
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function consume({ request: Struct }): int;

    /**
     * Cancel consumers and close the session
     * @node industrial_amqp_disconnect @alias industrialAmqpDisconnect
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function disconnect({ request: Struct }): bool;

    /**
     * Publish with routing checks and publisher confirmations using AMQP 0-9-1
     * @node industrial_amqp_publish @alias industrialAmqpPublish
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function publish({ request: Struct }): bool;
}

declare namespace cifx {
    // === Industrial/cifX Controller ===

    /**
     * Open a configured Hilscher card for PROFINET or another cifX fieldbus through its native driver; installed controller firmware runs the bus
     * @node industrial_cifx_connect @alias industrialCifxConnect
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function connect({ request: Struct }): Struct;

    /**
     * Restore the original host and bus states, then close the controller channel and driver
     * @node industrial_cifx_disconnect @alias industrialCifxDisconnect
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function disconnect({ request: Struct }): bool;

    /**
     * Return driver version, firmware identity, and configured process-image areas
     * @node industrial_cifx_info @alias industrialCifxInfo
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function info({ request: Struct }): Struct;

    /**
     * Deliver input samples while controller firmware maintains fieldbus communication
     * @node industrial_cifx_poll @alias industrialCifxPoll
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function poll({ request: Struct }): int;

    /**
     * Read bounded input bytes from the configured controller process image
     * @node industrial_cifx_read @alias industrialCifxRead
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function read({ request: Struct }): int[];

    /**
     * Start or stop the configured controller fieldbus and return its state
     * @node industrial_cifx_set_bus @alias industrialCifxSetBus
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function setBus({ request: Struct }): Struct;

    /**
     * Read controller host-ready and bus-on state
     * @node industrial_cifx_state @alias industrialCifxState
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function state({ request: Struct }): Struct;

    /**
     * Update bounded output bytes in the configured controller process image
     * @node industrial_cifx_write @alias industrialCifxWrite
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function write({ request: Struct }): bool;
}

declare namespace cip {
    // === Industrial/EtherNet-IP ===

    /**
     * Open an EtherNet/IP explicit messaging session for Logix tags, with optional backplane routing
     * @node industrial_cip_connect @alias industrialCipConnect
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function connect({ request: Struct }): Struct;

    /**
     * Stop tag polling and close the EtherNet/IP session
     * @node industrial_cip_disconnect @alias industrialCipDisconnect
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function disconnect({ request: Struct }): bool;

    /**
     * Poll a Logix tag and call the handler with its typed value. This uses explicit reads rather than cyclic implicit I/O.
     * @node industrial_cip_poll @alias industrialCipPoll
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function poll({ request: Struct }): int;

    /**
     * Read a typed Logix tag through CIP explicit messaging
     * @node industrial_cip_read @alias industrialCipRead
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function read({ request: Struct }): Struct;

    /**
     * Write a typed Logix tag through CIP explicit messaging
     * @node industrial_cip_write @alias industrialCipWrite
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function write({ request: Struct }): bool;
}

declare namespace dds {
    // === Industrial/DDS ===

    /**
     * Open a DDS publisher or subscriber for a final-struct topic on a local executor. Configure the exact IDL type name and ordered fields.
     * @node industrial_dds_connect @alias industrialDdsConnect
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function connect({ request: Struct }): Struct;

    /**
     * Stop pending DDS operations and remove the topic endpoint and domain participant
     * @node industrial_dds_disconnect @alias industrialDdsDisconnect
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function disconnect({ request: Struct }): bool;

    /**
     * Publish a JSON object matching the session's DDS schema. Requires a discovered reader; reliable publication waits for acknowledgments.
     * @node industrial_dds_publish @alias industrialDdsPublish
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function publish({ request: Struct }): bool;

    /**
     * Take one sample or instance lifecycle notification from a subscriber session, up to the configured timeout
     * @node industrial_dds_receive @alias industrialDdsReceive
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function receive({ request: Struct }): Struct;

    /**
     * Call the handler for each typed DDS sample or instance lifecycle notification until cancellation or the sample limit
     * @node industrial_dds_subscribe @alias industrialDdsSubscribe
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function subscribe({ request: Struct }): int;

    /**
     * Wait for a compatible DDS endpoint in the configured domain and topic, up to the session timeout
     * @node industrial_dds_wait_for_match @alias industrialDdsWaitForMatch
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function wait_for_match({ request: Struct }): int;
}

declare namespace ethercat {
    // === Industrial/EtherCAT ===

    /**
     * Read the latest process image, working counter, and cycle timing from the running master
     * @node industrial_ethercat_read @alias industrialEthercatRead
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function read({ request: Struct }): Struct;

    /**
     * Discover devices and start cyclic process data on a dedicated Linux or macOS interface. Requires raw network access. Uses EEPROM PDO mapping and optional PRE-OP SDO configuration.
     * @node industrial_ethercat_start @alias industrialEthercatStart
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function start({ request: Struct }): Struct;

    /**
     * Stop process-data sampling and return devices to INIT before closing the raw interface
     * @node industrial_ethercat_stop @alias industrialEthercatStop
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function stop({ request: Struct }): bool;

    /**
     * Call the handler with the latest process image at the requested interval. The cyclic worker continues while the handler runs; intermediate snapshots are coalesced.
     * @node industrial_ethercat_watch @alias industrialEthercatWatch
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function watch({ request: Struct }): int;

    /**
     * Update a bounded range of mapped PDO bytes and wait for the next successful bus exchange
     * @node industrial_ethercat_write @alias industrialEthercatWrite
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function write({ request: Struct }): bool;
}

declare namespace hart {
    // === Industrial/HART ===

    /**
     * Execute a HART command once and return response code, device status and binary payload
     * @node industrial_hart_command @alias industrialHartCommand
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function command({ request: Struct }): Struct;

    /**
     * Open a serial HART modem, transparent TCP gateway or HART-IP version 1 session
     * @node industrial_hart_connect @alias industrialHartConnect
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function connect({ request: Struct }): Struct;

    /**
     * Stop polling, close the device channel and terminate a HART-IP session
     * @node industrial_hart_disconnect @alias industrialHartDisconnect
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function disconnect({ request: Struct }): bool;

    /**
     * Poll a known read-only HART command and run a referenced handler for each response
     * @node industrial_hart_poll @alias industrialHartPoll
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function poll({ request: Struct }): int;
}

declare namespace inspection {
    // === AI/ML/Sensors ===

    /**
     * Capture a complete GigE Vision frame and preserve camera and host timestamps
     * @node ml_capture_genicam_frame @alias mlCaptureGenicamFrame
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function captureGenicam({ request: Struct }): Struct;

    /**
     * Decode signed, unsigned or floating-point registers with explicit byte and word order
     * @node ml_decode_sensor_registers @alias mlDecodeSensorRegisters
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function decodeRegisters({ request: Struct }): Struct[];

    /**
     * Read Modbus TCP input/holding registers or discrete inputs with bounded timeout
     * @node ml_read_modbus_sensor @alias mlReadModbusSensor
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function readModbus({ request: Struct }): Struct;

    /**
     * Read timestamped values over a verified OPC UA session with app certificates or a custom identity and server trust
     * @node ml_read_opcua_sensor @alias mlReadOpcuaSensor
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function readOpcUa({ request: Struct }): Struct[];
}

declare namespace iolink {
    // === Industrial/IO-Link ===

    /**
     * Connect to a master's IO-Link JSON Integration 2.0 REST API and discover its devices
     * @node industrial_iolink_connect @alias industrialIolinkConnect
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function connect({ request: Struct }): Struct;

    /**
     * Stop IO-Link polling and release the HTTP session
     * @node industrial_iolink_disconnect @alias industrialIolinkDisconnect
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function disconnect({ request: Struct }): bool;

    /**
     * Call the handler with device_alias and process data at an interval using the master's JSON 2.0 API
     * @node industrial_iolink_poll @alias industrialIolinkPoll
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function poll({ request: Struct }): int;

    /**
     * Read input process bytes, validity, and digital pin values through the master's JSON 2.0 API
     * @node industrial_iolink_read @alias industrialIolinkRead
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function read({ request: Struct }): Struct;

    /**
     * Read raw indexed service data by index and optional subindex through the master's JSON 2.0 API
     * @node industrial_iolink_read_parameter @alias industrialIolinkReadParameter
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function read_parameter({ request: Struct }): int[];

    /**
     * Write output process bytes or digital pin values through the master's JSON 2.0 API
     * @node industrial_iolink_write @alias industrialIolinkWrite
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function write({ request: Struct }): bool;

    /**
     * Write raw indexed service data by index and optional subindex through the master's JSON 2.0 API
     * @node industrial_iolink_write_parameter @alias industrialIolinkWriteParameter
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function write_parameter({ request: Struct }): bool;
}

declare namespace iroh {
    // === Industrial/Iroh ===

    /**
     * Create a local Iroh endpoint for Flow-Like framed messages. Returns its public identity and addresses.
     * @node industrial_iroh_bind @alias industrialIrohBind
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function bind({ request: Struct }): Struct;

    /**
     * Stop accepting peers and close all connections on this endpoint
     * @node industrial_iroh_close_endpoint @alias industrialIrohCloseEndpoint
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function close_endpoint({ request: Struct }): bool;

    /**
     * Connect to a peer identity using the flow-like/messages/2 application protocol
     * @node industrial_iroh_connect @alias industrialIrohConnect
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function connect({ request: Struct }): Struct;

    /**
     * Stop receiving and close an Iroh peer connection
     * @node industrial_iroh_disconnect @alias industrialIrohDisconnect
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function disconnect({ request: Struct }): bool;

    /**
     * Accept allowlisted peers and run the connection handler in order. Each connection closes when its handler returns.
     * @node industrial_iroh_listen @alias industrialIrohListen
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function listen({ request: Struct }): int;

    /**
     * Call the handler for each framed message with its authenticated peer ID and payload
     * @node industrial_iroh_receive @alias industrialIrohReceive
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function receive({ request: Struct }): int;

    /**
     * Send one bounded binary message and wait for the peer to acknowledge receipt of the complete frame
     * @node industrial_iroh_send @alias industrialIrohSend
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function send({ request: Struct }): bool;
}

declare namespace kafka {
    // === Industrial/Kafka ===

    /**
     * Open a Kafka and Redpanda session with bounded connection timeout
     * @node industrial_kafka_connect @alias industrialKafkaConnect
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function connect({ request: Struct }): Struct;

    /**
     * Run a referenced handler for each message and acknowledge durable deliveries only after successful processing
     * @node industrial_kafka_consume @alias industrialKafkaConsume
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function consume({ request: Struct }): int;

    /**
     * Cancel consumers and close the session
     * @node industrial_kafka_disconnect @alias industrialKafkaDisconnect
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function disconnect({ request: Struct }): bool;

    /**
     * Publish a Kafka record and await the broker delivery result
     * @node industrial_kafka_publish @alias industrialKafkaPublish
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function publish({ request: Struct }): Struct;
}

declare namespace modbus {
    // === Industrial/Modbus ===

    /**
     * Open a persistent Modbus TCP or serial RTU session
     * @node industrial_modbus_connect @alias industrialModbusConnect
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function connect({ request: Struct }): Struct;

    /**
     * Stop polling and close a Modbus session
     * @node industrial_modbus_disconnect @alias industrialModbusDisconnect
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function disconnect({ request: Struct }): bool;

    /**
     * Read values at an interval and call the referenced handler with each sample or change
     * @node industrial_modbus_poll @alias industrialModbusPoll
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function poll({ request: Struct }): int;

    /**
     * Read coils, discrete inputs, or registers over an existing session
     * @node industrial_modbus_read @alias industrialModbusRead
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function read({ request: Struct }): Struct;

    /**
     * Write coils or holding registers over an existing session
     * @node industrial_modbus_write @alias industrialModbusWrite
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function write({ request: Struct }): bool;
}

declare namespace nats {
    // === Industrial/NATS ===

    /**
     * Open a NATS and JetStream session with bounded connection timeout
     * @node industrial_nats_connect @alias industrialNatsConnect
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function connect({ request: Struct }): Struct;

    /**
     * Run a referenced handler for each message and acknowledge durable deliveries only after successful processing
     * @node industrial_nats_consume @alias industrialNatsConsume
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function consume({ request: Struct }): int;

    /**
     * Cancel consumers and close the session
     * @node industrial_nats_disconnect @alias industrialNatsDisconnect
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function disconnect({ request: Struct }): bool;

    /**
     * Confirms JetStream persistence or flushes a Core NATS publication
     * @node industrial_nats_publish @alias industrialNatsPublish
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function publish({ request: Struct }): Struct;
}

declare namespace opcua {
    // === Industrial/OPC UA ===

    /**
     * Read a bounded page of forward node references and report truncation
     * @node industrial_opcua_browse @alias industrialOpcuaBrowse
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function browse({ request: Struct }): Struct;

    /**
     * Open an OPC UA session with app certificates or a custom application identity and server trust
     * @node industrial_opcua_connect @alias industrialOpcuaConnect
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function connect({ request: Struct }): Struct;

    /**
     * Stop subscriptions and close an OPC UA session
     * @node industrial_opcua_disconnect @alias industrialOpcuaDisconnect
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function disconnect({ request: Struct }): bool;

    /**
     * Read scalar node values with quality status and timestamps
     * @node industrial_opcua_read @alias industrialOpcuaRead
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function read({ request: Struct }): Struct[];

    /**
     * Monitor scalar values and call the referenced handler on data changes
     * @node industrial_opcua_subscribe @alias industrialOpcuaSubscribe
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function subscribe({ request: Struct }): int;

    /**
     * Write explicitly typed scalar node values and return each write status
     * @node industrial_opcua_write @alias industrialOpcuaWrite
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function write({ request: Struct }): Struct[];
}

declare namespace opcua_pubsub {
    // === Industrial/OPC UA PubSub ===

    /**
     * Open a local UADP UDP publisher or subscriber with configured scalar field metadata. Supports plaintext or PubSub-Aes256-CTR group keys supplied through the security configuration. Requires a local executor.
     * @node industrial_opcua_pubsub_connect @alias industrialOpcuaPubsubConnect
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function connect({ request: Struct }): Struct;

    /**
     * Cancel pending receives and handlers, leave the multicast group, and close the UDP socket
     * @node industrial_opcua_pubsub_disconnect @alias industrialOpcuaPubsubDisconnect
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function disconnect({ request: Struct }): bool;

    /**
     * Send one typed UADP key frame and return its sequence number. UDP delivery is not acknowledged.
     * @node industrial_opcua_pubsub_publish @alias industrialOpcuaPubsubPublish
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function publish({ request: Struct }): int;

    /**
     * Receive one matching UADP DataSet within the configured timeout and validate its field types and metadata version
     * @node industrial_opcua_pubsub_receive @alias industrialOpcuaPubsubReceive
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function receive({ request: Struct }): Struct;

    /**
     * Call the referenced handler for matching scalar UADP key frames. Timeout or malformed matching data stops the listener.
     * @node industrial_opcua_pubsub_subscribe @alias industrialOpcuaPubsubSubscribe
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function subscribe({ request: Struct }): int;
}

declare namespace profibus {
    // === Industrial/PROFIBUS ===

    /**
     * Start native cyclic DP-V0 exchange on an RS-485 serial adapter using device GSD parameter bytes
     * @node industrial_profibus_connect @alias industrialProfibusConnect
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function connect({ request: Struct }): Struct;

    /**
     * Attempt a clear cycle, stop cyclic traffic, and close the serial device
     * @node industrial_profibus_disconnect @alias industrialProfibusDisconnect
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function disconnect({ request: Struct }): bool;

    /**
     * Deliver the newest process-image snapshot while cyclic bus traffic continues independently
     * @node industrial_profibus_listen @alias industrialProfibusListen
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function listen({ request: Struct }): int;

    /**
     * Read current process images and peripheral diagnostics from the running master
     * @node industrial_profibus_read @alias industrialProfibusRead
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function read({ request: Struct }): Struct;

    /**
     * Switch the DP master between stop, clear, and operate
     * @node industrial_profibus_set_mode @alias industrialProfibusSetMode
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function setMode({ request: Struct }): bool;

    /**
     * Update a peripheral output process image for the next bus cycle
     * @node industrial_profibus_write @alias industrialProfibusWrite
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function write({ request: Struct }): bool;
}

declare namespace redis {
    // === Industrial/Redis Streams ===

    /**
     * Open a Redis Streams session with bounded connection timeout
     * @node industrial_redis_connect @alias industrialRedisConnect
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function connect({ request: Struct }): Struct;

    /**
     * Run a referenced handler for each message and acknowledge durable deliveries only after successful processing
     * @node industrial_redis_consume @alias industrialRedisConsume
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function consume({ request: Struct }): int;

    /**
     * Cancel consumers and close the session
     * @node industrial_redis_disconnect @alias industrialRedisDisconnect
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function disconnect({ request: Struct }): bool;

    /**
     * Append binary fields to a Redis stream and return its generated entry ID
     * @node industrial_redis_publish @alias industrialRedisPublish
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function publish({ request: Struct }): string;
}

declare namespace sparkplug {
    // === Industrial/Sparkplug ===

    /**
     * Decode MQTT bytes into the Sparkplug protobuf JSON representation
     * @node sparkplug_decode @alias sparkplugDecode
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function decode({ request: Struct }): Struct;

    /**
     * Encode a Sparkplug protobuf payload, or a host STATE JSON payload, for MQTT
     * @node sparkplug_encode @alias sparkplugEncode
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function encode({ request: Struct }): Struct;

    /**
     * Track one edge node's online state and detect sequence gaps that require rebirth
     * @node sparkplug_observe @alias sparkplugObserve
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function observe({ request: Struct }): Struct;

    /**
     * Advance persisted bdSeq and prepare the MQTT Last Will; install it before connecting with a clean session
     * @node sparkplug_prepare_session @alias sparkplugPrepareSession
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function prepareSession({ request: Struct }): Struct;

    /**
     * Build birth, data, death, or rebirth messages with one shared sequence; publish the returned messages in order
     * @node sparkplug_transition @alias sparkplugTransition
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function transition({ request: Struct }): Struct;
}

declare namespace zenoh {
    // === Industrial/Zenoh ===

    /**
     * Open a Zenoh peer or client session on a local executor
     * @node industrial_zenoh_connect @alias industrialZenohConnect
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function connect({ request: Struct }): Struct;

    /**
     * Publish a delete sample for a Zenoh key
     * @node industrial_zenoh_delete @alias industrialZenohDelete
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function delete({ request: Struct }): bool;

    /**
     * Stop subscriptions and close the Zenoh session
     * @node industrial_zenoh_disconnect @alias industrialZenohDisconnect
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function disconnect({ request: Struct }): bool;

    /**
     * Publish bytes and their encoding on a Zenoh key
     * @node industrial_zenoh_publish @alias industrialZenohPublish
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function publish({ request: Struct }): bool;

    /**
     * Collect bounded replies using a selector and timeout. Fails if the 128-reply receive queue overflows.
     * @node industrial_zenoh_query @alias industrialZenohQuery
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function query({ request: Struct }): Struct[];

    /**
     * Call the handler with each key, payload, encoding, and timestamp. Fails if the 128-sample receive queue overflows.
     * @node industrial_zenoh_subscribe @alias industrialZenohSubscribe
     * @param request — Inputs and configuration
     * @returns result — Operation result
     * @impure has side effects / drives control flow
     */
    function subscribe({ request: Struct }): int;
}
