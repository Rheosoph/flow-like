// Package sdk provides the Go SDK for Flow-Like WASM nodes.
//
// It mirrors the Rust/AssemblyScript SDK types and patterns, targeting TinyGo
// compilation to wasm32. Host imports use //go:wasmimport directives.
//
// The SDK is split across multiple files:
//   - types.go:   JSON-serializable types (NodeDefinition, PinDefinition, etc.)
//   - host.go:    Raw host import declarations and Go wrapper functions
//   - context.go: Context struct with high-level helpers
//   - memory.go:  alloc/dealloc exports and memory helpers
//   - sdk.go:     (this file) ParseInput, SerializeDefinition, SerializeResult
package sdk

// ParseInput deserializes an ExecutionInput from wasm memory at the given pointer.
func ParseInput(ptr uint32, length uint32) ExecutionInput {
	jsonStr := ptrToString(ptr, length)
	return parseExecutionInputJSON(jsonStr)
}

// SerializeDefinition serializes a NodeDefinition to JSON and returns a packed i64.
func SerializeDefinition(def NodeDefinition) int64 {
	return PackResult(def.ToJSON())
}

// SerializeResult serializes an ExecutionResult to JSON and returns a packed i64.
func SerializeResult(result ExecutionResult) int64 {
	return PackResult(result.ToJSON())
}

// parseExecutionInputJSON is a minimal JSON parser for ExecutionInput.
// It avoids importing encoding/json (which bloats the wasm binary under TinyGo).
func parseExecutionInputJSON(s string) ExecutionInput {
	input := ExecutionInput{
		Inputs:   make(map[string]string),
		LogLevel: 1,
	}
	idx := 0

	skipWhitespace := func() {
		for idx < len(s) && (s[idx] == ' ' || s[idx] == '\t' || s[idx] == '\n' || s[idx] == '\r') {
			idx++
		}
	}

	readString := func() string {
		if idx >= len(s) || s[idx] != '"' {
			return ""
		}
		idx++ // skip opening quote
		start := idx
		for idx < len(s) && s[idx] != '"' {
			if s[idx] == '\\' {
				idx++
			}
			idx++
		}
		result := s[start:idx]
		if idx < len(s) {
			idx++ // skip closing quote
		}
		return result
	}

	// readValue reads a JSON value as raw string (string, number, bool, object, array)
	var readValue func() string
	readValue = func() string {
		skipWhitespace()
		if idx >= len(s) {
			return ""
		}
		switch s[idx] {
		case '"':
			v := readString()
			return `"` + v + `"`
		case '{':
			depth := 0
			start := idx
			for idx < len(s) {
				if s[idx] == '{' {
					depth++
				} else if s[idx] == '}' {
					depth--
					if depth == 0 {
						idx++
						return s[start:idx]
					}
				} else if s[idx] == '"' {
					idx++
					for idx < len(s) && s[idx] != '"' {
						if s[idx] == '\\' {
							idx++
						}
						idx++
					}
				}
				idx++
			}
			return s[start:idx]
		case '[':
			depth := 0
			start := idx
			for idx < len(s) {
				if s[idx] == '[' {
					depth++
				} else if s[idx] == ']' {
					depth--
					if depth == 0 {
						idx++
						return s[start:idx]
					}
				} else if s[idx] == '"' {
					idx++
					for idx < len(s) && s[idx] != '"' {
						if s[idx] == '\\' {
							idx++
						}
						idx++
					}
				}
				idx++
			}
			return s[start:idx]
		default:
			start := idx
			for idx < len(s) && s[idx] != ',' && s[idx] != '}' && s[idx] != ']' &&
				s[idx] != ' ' && s[idx] != '\t' && s[idx] != '\n' && s[idx] != '\r' {
				idx++
			}
			return s[start:idx]
		}
	}

	skipWhitespace()
	if idx >= len(s) || s[idx] != '{' {
		return input
	}
	idx++ // skip {

	for idx < len(s) {
		skipWhitespace()
		if idx >= len(s) || s[idx] == '}' {
			break
		}
		if s[idx] == ',' {
			idx++
			continue
		}
		key := readString()
		skipWhitespace()
		if idx < len(s) && s[idx] == ':' {
			idx++
		}
		skipWhitespace()

		switch key {
		case "node_id":
			input.NodeID = readString()
		case "node_name":
			input.NodeName = readString()
		case "run_id":
			input.RunID = readString()
		case "app_id":
			input.AppID = readString()
		case "board_id":
			input.BoardID = readString()
		case "user_id":
			input.UserID = readString()
		case "stream_state":
			v := readValue()
			input.StreamState = v == "true"
		case "log_level":
			v := readValue()
			if len(v) == 1 && v[0] >= '0' && v[0] <= '9' {
				input.LogLevel = v[0] - '0'
			}
		case "inputs":
			skipWhitespace()
			if idx < len(s) && s[idx] == '{' {
				idx++
				for idx < len(s) {
					skipWhitespace()
					if idx >= len(s) || s[idx] == '}' {
						idx++
						break
					}
					if s[idx] == ',' {
						idx++
						continue
					}
					iKey := readString()
					skipWhitespace()
					if idx < len(s) && s[idx] == ':' {
						idx++
					}
					iVal := readValue()
					input.Inputs[iKey] = iVal
				}
			} else {
				readValue()
			}
		default:
			readValue()
		}
	}

	return input
}
