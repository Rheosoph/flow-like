# syntax=docker/dockerfile:1.8
# Multi-architecture release manifest for opentelemetry-collector-contrib 0.148.0.
FROM ghcr.io/open-telemetry/opentelemetry-collector-releases/opentelemetry-collector-contrib@sha256:8164eab2e6bca9c9b0837a8d2f118a6618489008a839db7f9d6510e66be3923c

COPY --chown=10001:10001 collector.yaml /etc/otelcol-contrib/config.yaml

USER 10001:10001
EXPOSE 4317 13133

ENTRYPOINT ["/otelcol-contrib"]
CMD ["--config=/etc/otelcol-contrib/config.yaml"]
