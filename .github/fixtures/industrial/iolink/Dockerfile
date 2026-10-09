FROM stoplight/prism:5.14.2@sha256:87972b6cf73da2c0339fca18f362b6f93118152d652c970fab93ed13cfd55bae

# The official 2.0.0 OpenAPI document is the contract oracle. This is a stateless
# HTTP contract fixture; it does not emulate a physical IO-Link master.
ADD --checksum=sha256:be5998cd2f30b88c747a9169fe23d4338666f4a896d7de16f335c1cc4aca1889 https://raw.githubusercontent.com/iolinkcommunity/JSON_for_IO-Link/66662fb3570c91e36f0af37a12070033788a34ba/JSON_for_IO-Link_unmerged.yaml /spec/api.yaml
ADD --checksum=sha256:f578eae684017c856f33bccba9e8ac516c30dc2923f5e412caeb7ed09186d2a9 https://raw.githubusercontent.com/iolinkcommunity/JSON_for_IO-Link/66662fb3570c91e36f0af37a12070033788a34ba/schemas.yaml /spec/schemas.yaml
ADD --checksum=sha256:957a9c56c2ef36df165a148639516128445ba2e66a5a22dfa2bd0cf0f5333edc https://raw.githubusercontent.com/iolinkcommunity/JSON_for_IO-Link/66662fb3570c91e36f0af37a12070033788a34ba/examples.yaml /spec/examples.yaml
ADD --checksum=sha256:6fd4d7d12ce29cd6a2010a960a81752e54317287160805074d3eb3bb796f5ad9 https://raw.githubusercontent.com/iolinkcommunity/JSON_for_IO-Link/66662fb3570c91e36f0af37a12070033788a34ba/LICENSE /spec/LICENSE

# Prism ignores the variable server basePath. Apply the documented iolink/v2
# prefix to operation paths, preserving the upstream schemas and examples.
USER root
RUN sed -i 's|^  "/|  "/iolink/v2/|' /spec/api.yaml && chmod 644 /spec/*
USER 1000
EXPOSE 4010
HEALTHCHECK --interval=2s --timeout=5s --retries=30 CMD node -e "require('http').get('http://127.0.0.1:4010/iolink/v2/devices', r => process.exit(r.statusCode === 200 ? 0 : 1)).on('error', () => process.exit(1))"
CMD ["mock", "-h", "0.0.0.0", "-p", "4010", "--errors", "/spec/api.yaml"]
