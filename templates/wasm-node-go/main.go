// Flow-Like WASM Node Template (Go / TinyGo) — Component Model
//
// Uses WASI Preview 2 (Component Model) for full TCP/UDP/DNS support.
//
// Build:
//   mise run build
//
// Or manually:
//   wit-bindgen-go generate --world flow-like-node --out gen ./wit
//   go mod tidy
//   tinygo build -o node.wasm -target wasip2 -no-debug ./
package main

import (
	"strconv"
	"strings"

	flowlikenode "github.com/example/flow-like-wasm-node/gen/flow-like/node/flow-like-node"
)

func init() {
	flowlikenode.Exports.GetNode = func() string {
		return buildDefinition().ToJSON()
	}
	flowlikenode.Exports.GetNodes = func() string {
		return "[" + buildDefinition().ToJSON() + "]"
	}
	flowlikenode.Exports.Run = func(input string) string {
		ctx := NewContext()
		return handleRun(ctx).ToJSON()
	}
	flowlikenode.Exports.GetABIVersion = func() uint32 {
		return ABIVersion
	}
}

func main() {}

// ── Node Definition ─────────────────────────────────────────────────────

func buildDefinition() NodeDefinition {
	def := NewNodeDefinition(
		"my_custom_node_go",
		"My Custom Node (Go)",
		"A template WASM node built with Go / TinyGo (Component Model)",
		"Custom/WASM",
	)
	def.AddPermission("streaming")

	def.AddPin(InputExecPin("exec"))
	def.AddPin(InputPin("input_text", "Input Text", "Text to process", "String").WithDefault(`""`))
	def.AddPin(InputPin("multiplier", "Multiplier", "Number of times to repeat", "I64").WithDefault("1"))

	def.AddPin(OutputExecPin("exec_out"))
	def.AddPin(OutputPin("output_text", "Output Text", "Processed text", "String"))
	def.AddPin(OutputPin("char_count", "Character Count", "Number of characters in output", "I64"))

	return def
}

// ── Run Handler ─────────────────────────────────────────────────────────

func handleRun(ctx *Context) ExecutionResult {
	inputText := ctx.GetString("input_text", "")
	multiplier := ctx.GetI64("multiplier", 1)

	ctx.Debug("Processing: '" + inputText + "' x " + strconv.FormatInt(multiplier, 10))

	var b strings.Builder
	for i := int64(0); i < multiplier; i++ {
		b.WriteString(inputText)
	}
	outputText := b.String()
	charCount := len(outputText)

	ctx.StreamText("Generated " + strconv.Itoa(charCount) + " characters")

	ctx.SetOutput("output_text", jsonQuote(outputText))
	ctx.SetOutput("char_count", strconv.Itoa(charCount))

	return ctx.Success()
}

// ── Network Example (Component Model enables direct TCP/UDP/DNS) ────────
//
// With Component Model (wasip2), net.Dial works for raw TCP connections:
//
//   conn, err := net.Dial("tcp", "example.com:80")
//   if err != nil { return }
//   defer conn.Close()
//   conn.Write([]byte("GET / HTTP/1.0\r\nHost: example.com\r\n\r\n"))
//   buf := make([]byte, 4096)
//   n, _ := conn.Read(buf)
//   response := string(buf[:n])
