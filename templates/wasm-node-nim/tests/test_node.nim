## Tests for the Nim WASM node (run natively, not in WASM)
##
## These tests verify the node definition and execution logic
## without needing the WASM host runtime.

import std/[unittest, tables, strutils]

# Import SDK types directly (not the host module which needs WASM imports)
import ../src/sdk {.all.}

suite "Node Definition":
  test "definition has correct name":
    let def = initNodeDefinition()
    check def.abiVersion == 1

  test "pin builder creates input pin":
    let pin = inputPin("test", "Test", "A test pin", String)
    check pin.name == "test"
    check pin.friendlyName == "Test"
    check pin.pinType == Input
    check pin.dataType == String

  test "pin builder creates output pin":
    let pin = outputPin("out", "Output", "An output pin", I64)
    check pin.name == "out"
    check pin.pinType == Output
    check pin.dataType == I64

  test "pin withDefault sets default value":
    let pin = inputPin("x", "X", "desc", String).withDefault("\"hello\"")
    check pin.defaultValue == "\"hello\""

  test "node definition JSON serialization":
    var def = initNodeDefinition()
    def.name = "test_node"
    def.friendlyName = "Test Node"
    def.description = "A test"
    def.category = "Test"
    def.addPin inputPin("exec", "Exec", "Go", Exec)
    def.addPin outputPin("exec_out", "Done", "Done", Exec)

    let json = def.toJson()
    check "\"name\":\"test_node\"" in json
    check "\"friendly_name\":\"Test Node\"" in json
    check "\"abi_version\":1" in json

  test "node permissions in JSON":
    var def = initNodeDefinition()
    def.name = "perm_node"
    def.friendlyName = "Perm"
    def.description = "d"
    def.category = "c"
    def.addPermission("http")
    def.addPermission("streaming")

    let json = def.toJson()
    check "\"permissions\":" in json
    check "\"http\"" in json
    check "\"streaming\"" in json

suite "Execution Result":
  test "ok result serialization":
    var res = ok()
    let json = res.toJson()
    check "\"outputs\":{}" in json
    check "\"pending\":false" in json

  test "fail result has error":
    var res = fail("something broke")
    let json = res.toJson()
    check "\"error\":\"something broke\"" in json

  test "set output and exec":
    var res = ok()
    res.setOutput("name", "\"hello\"")
    res.exec("exec_out")
    let json = res.toJson()
    check "\"name\":\"hello\"" in json
    check "\"exec_out\"" in json

suite "JSON Utilities":
  test "jsonQuote escapes special characters":
    check jsonQuote("hello") == "\"hello\""
    check jsonQuote("a\"b") == "\"a\\\"b\""
    check jsonQuote("a\\b") == "\"a\\\\b\""

  test "jsonString wraps string":
    check jsonString("test") == "\"test\""

suite "Input Parsing":
  test "parse simple execution input":
    let raw = """{"node_id":"n1","node_name":"test","run_id":"r1","app_id":"a1","board_id":"b1","user_id":"u1","stream_state":false,"log_level":1,"inputs":{"text":"\"hello\"","count":"42"}}"""
    let inp = parseInput(raw)
    check inp.nodeId == "n1"
    check inp.nodeName == "test"
    check inp.runId == "r1"
    check inp.logLevel == 1
    check inp.streamState == false
    check inp.inputs["text"] == "\"hello\""
    check inp.inputs["count"] == "42"
