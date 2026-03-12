---
title: Go WASM Nodes
description: Create custom WASM nodes using Go and TinyGo with the WASM Component Model
sidebar:
  order: 2
  badge:
    text: Component Model
    variant: success
---

Go can compile to WASM using **TinyGo** (recommended for smaller binaries) or standard Go.

## Prerequisites

```bash
# Install TinyGo (recommended)
# macOS
brew install tinygo

# Linux
wget https://github.com/tinygo-org/tinygo/releases/download/v0.32.0/tinygo_0.32.0_amd64.deb
sudo dpkg -i tinygo_0.32.0_amd64.deb

# Verify installation
tinygo version
```

## Project Setup

```bash
mkdir my-custom-node
cd my-custom-node
go mod init my-custom-node
```

## Template Code

```go title="main.go"
package main

import (
	"encoding/json"
	"strings"
	"unsafe"
)

// Node definition structures
type NodeDefinition struct {
	Name         string          `json:"name"`
	FriendlyName string          `json:"friendly_name"`
	Description  string          `json:"description"`
	Category     string          `json:"category"`
	Icon         string          `json:"icon,omitempty"`
	Pins         []PinDefinition `json:"pins"`
	Scores       *NodeScores     `json:"scores,omitempty"`
}

type PinDefinition struct {
	Name         string      `json:"name"`
	FriendlyName string      `json:"friendly_name"`
	Description  string      `json:"description"`
	PinType      string      `json:"pin_type"`
	DataType     string      `json:"data_type"`
	DefaultValue interface{} `json:"default_value,omitempty"`
}

type NodeScores struct {
	Privacy     uint8 `json:"privacy"`
	Security    uint8 `json:"security"`
	Performance uint8 `json:"performance"`
	Governance  uint8 `json:"governance"`
	Reliability uint8 `json:"reliability"`
	Cost        uint8 `json:"cost"`
}

// Execution context
type ExecutionContext struct {
	Inputs map[string]interface{} `json:"inputs"`
}

type ExecutionResult struct {
	Outputs map[string]interface{} `json:"outputs"`
	Error   string                 `json:"error,omitempty"`
}

// Memory management for WASM
var resultBuffer []byte

//export get_node
func get_node() unsafe.Pointer {
	node := NodeDefinition{
		Name:         "wasm_go_uppercase",
		FriendlyName: "Uppercase (Go)",
		Description:  "Converts a string to uppercase using Go",
		Category:     "Custom/Text",
		Icon:         "/flow/icons/text.svg",
		Pins: []PinDefinition{
			{
				Name:         "exec_in",
				FriendlyName: "▶",
				Description:  "Trigger execution",
				PinType:      "Input",
				DataType:     "Execution",
			},
			{
				Name:         "exec_out",
				FriendlyName: "▶",
				Description:  "Continue execution",
				PinType:      "Output",
				DataType:     "Execution",
			},
			{
				Name:         "input",
				FriendlyName: "Input",
				Description:  "The string to convert",
				PinType:      "Input",
				DataType:     "String",
				DefaultValue: "",
			},
			{
				Name:         "output",
				FriendlyName: "Output",
				Description:  "The uppercase string",
				PinType:      "Output",
				DataType:     "String",
			},
		},
		Scores: &NodeScores{
			Privacy:     0,
			Security:    0,
			Performance: 1,
			Governance:  0,
			Reliability: 0,
			Cost:        0,
		},
	}

	jsonBytes, _ := json.Marshal(node)
	resultBuffer = jsonBytes
	return unsafe.Pointer(&resultBuffer[0])
}

//export run
func run(contextPtr unsafe.Pointer, contextLen uint32) unsafe.Pointer {
	// Read context from memory
	contextBytes := unsafe.Slice((*byte)(contextPtr), contextLen)

	var context ExecutionContext
	if err := json.Unmarshal(contextBytes, &context); err != nil {
		return errorResult("Failed to parse context: " + err.Error())
	}

	// Get input value
	input := ""
	if val, ok := context.Inputs["input"]; ok {
		if str, ok := val.(string); ok {
			input = str
		}
	}

	// Execute logic
	output := strings.ToUpper(input)

	// Return result
	result := ExecutionResult{
		Outputs: map[string]interface{}{
			"output": output,
		},
	}

	jsonBytes, _ := json.Marshal(result)
	resultBuffer = jsonBytes
	return unsafe.Pointer(&resultBuffer[0])
}

func errorResult(message string) unsafe.Pointer {
	result := ExecutionResult{
		Outputs: make(map[string]interface{}),
		Error:   message,
	}
	jsonBytes, _ := json.Marshal(result)
	resultBuffer = jsonBytes
	return unsafe.Pointer(&resultBuffer[0])
}

func main() {}
```

## Build with TinyGo

```bash
tinygo build -o my-custom-node.wasm -target=wasip1 -opt=s main.go
```

### Build Options

| Flag | Description |
|------|-------------|
| `-target=wasip1` | WASI Preview 1 target |
| `-opt=s` | Optimize for size |
| `-opt=2` | Optimize for speed |
| `-no-debug` | Remove debug info |

## Build with Standard Go

Standard Go produces larger binaries but has full language support:

```bash
GOOS=wasip1 GOARCH=wasm go build -o my-custom-node.wasm main.go
```

## Size Comparison

| Compiler | Typical Size |
|----------|--------------|
| TinyGo `-opt=s` | ~50-200 KB |
| Standard Go | ~2-5 MB |

## Install

```bash
cp my-custom-node.wasm ~/.flow-like/nodes/
```

## Limitations (TinyGo)

TinyGo doesn't support all Go features:

- ❌ `reflect` (limited support)
- ❌ `cgo`
- ❌ Some stdlib packages
- ✅ Most common packages work

See [TinyGo compatibility](https://tinygo.org/docs/reference/lang-support/) for details.

## Advanced: Using Third-Party Packages

```go
import (
    "github.com/google/uuid"
)

func generateID() string {
    return uuid.New().String()
}
```

Build:

```bash
go get github.com/google/uuid
tinygo build -o node.wasm -target=wasip1 main.go
```

## Related

→ [WASM Nodes Overview](/dev/wasm-nodes/overview/)
→ [Rust Template](/dev/wasm-nodes/rust/)
→ [TypeScript Template](/dev/wasm-nodes/typescript/)
