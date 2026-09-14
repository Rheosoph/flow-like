// Executed only by Docker's otel-extension-smoke target, never shipped to Lambda.
package main

import (
	"context"
	"encoding/hex"
	"fmt"
	"time"

	collector "go.opentelemetry.io/proto/otlp/collector/trace/v1"
	common "go.opentelemetry.io/proto/otlp/common/v1"
	resource "go.opentelemetry.io/proto/otlp/resource/v1"
	trace "go.opentelemetry.io/proto/otlp/trace/v1"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
)

func main() {
	conn, err := grpc.NewClient("127.0.0.1:4317", grpc.WithTransportCredentials(insecure.NewCredentials()))
	if err != nil {
		panic(err)
	}
	defer conn.Close()
	now := time.Now()
	traceID, _ := hex.DecodeString(fmt.Sprintf("%08x", now.Unix()) + "00112233445566778899aabb")
	spanID, _ := hex.DecodeString("8899aabbccddeeff")
	parentID, _ := hex.DecodeString("0011223344556677")
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	_, err = collector.NewTraceServiceClient(conn).Export(ctx, &collector.ExportTraceServiceRequest{
		ResourceSpans: []*trace.ResourceSpans{{
			Resource: &resource.Resource{Attributes: []*common.KeyValue{{
				Key: "service.name", Value: &common.AnyValue{Value: &common.AnyValue_StringValue{StringValue: "flow-like-api"}},
			}}},
			ScopeSpans: []*trace.ScopeSpans{{Spans: []*trace.Span{{
				TraceId: traceID, SpanId: spanID, ParentSpanId: parentID,
				Name: "GET /api/v1/smoke", Kind: trace.Span_SPAN_KIND_INTERNAL,
				StartTimeUnixNano: uint64(now.UnixNano()), EndTimeUnixNano: uint64(now.Add(time.Millisecond).UnixNano()),
			}}}},
		}},
	})
	if err != nil {
		panic(err)
	}
	fmt.Printf("1-%x-%x\n", traceID[:4], traceID[4:])
}
