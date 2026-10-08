import json
import unittest

import httpx

from flow_like import (
    EmbeddingAudioInput,
    EmbeddingImageInput,
    EmbeddingMultimodalInput,
    EmbeddingTextInput,
    EmbeddingVideoInput,
    FlowLikeClient,
)


SIGNED_URL = (
    "https://storage.googleapis.com/bucket/demo.mp4"
    "?X-Goog-Signature=a%2Fb&X-Goog-Expires=600"
)
CASES = [
    ("single text", "query: demo", ["query: demo"]),
    ("text batch", ["first", "second"], ["first", "second"]),
    ("structured text", EmbeddingTextInput(type="text", text="demo"), None),
    ("image", EmbeddingImageInput(type="image", image="aW1hZ2U="), None),
    (
        "audio",
        EmbeddingAudioInput(type="audio", audio="data:audio/wav;base64,YXVkaW8="),
        None,
    ),
    ("signed video", EmbeddingVideoInput(type="video", video=SIGNED_URL), None),
    (
        "combined input",
        EmbeddingMultimodalInput(
            type="multimodal",
            text="Demo: <|image|> <|audio|> <|video|>",
            image="https://example.com/product.jpg",
            audio="data:audio/wav;base64,YXVkaW8=",
            video=SIGNED_URL,
        ),
        None,
    ),
]
MIXED_BATCH = [
    "demo",
    EmbeddingAudioInput(type="audio", audio="data:audio/wav;base64,YXVkaW8="),
    EmbeddingVideoInput(type="video", video=SIGNED_URL),
]
CASES.extend([
    ("mixed batch", MIXED_BATCH, MIXED_BATCH),
    ("mixed sequence", tuple(MIXED_BATCH), MIXED_BATCH),
])


class EmbeddingsTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.requests = []
        self.client = FlowLikeClient(base_url="https://flow-like.example", pat="pat_test")
        transport = httpx.MockTransport(self.handle_request)
        self.client._client = httpx.Client(base_url=self.client._api_base, transport=transport)
        self.client._async_client = httpx.AsyncClient(
            base_url=self.client._api_base, transport=transport
        )

    async def asyncTearDown(self):
        await self.client.aclose()

    def handle_request(self, request):
        self.assertEqual(request.method, "POST")
        self.assertEqual(request.url.path, "/api/v1/embeddings/embed")
        payload = json.loads(request.content)
        self.requests.append(payload)
        return httpx.Response(200, json={
            "embeddings": [[index, 0.5] for index in range(len(payload["input"]))],
            "usage": {"prompt_tokens": 42, "total_tokens": 42},
            "usage_estimated": False,
            "usage_available": True,
        })

    def assert_result(self, result, expected):
        self.assertEqual(self.requests[-1], {
            "model": "embedding-bit",
            "input": expected,
            "embed_type": "document",
        })
        self.assertEqual(result.embeddings, [[index, 0.5] for index in range(len(expected))])
        self.assertEqual(result.usage.prompt_tokens, 42)
        self.assertEqual(result.usage.total_tokens, 42)
        self.assertFalse(result.usage_estimated)
        self.assertTrue(result.usage_available)

    def test_sync_preserves_media_and_batch_boundaries(self):
        for name, input, expected in CASES:
            with self.subTest(name=name):
                result = self.client.embed("embedding-bit", input, embed_type="document")
                self.assert_result(result, expected if expected is not None else [input])

    async def test_async_preserves_media_and_batch_boundaries(self):
        for name, input, expected in CASES:
            with self.subTest(name=name):
                result = await self.client.aembed("embedding-bit", input, embed_type="document")
                self.assert_result(result, expected if expected is not None else [input])

    async def test_text_requests_keep_query_default(self):
        self.client.embed("embedding-bit", "query")
        await self.client.aembed("embedding-bit", "query")
        for payload in self.requests:
            self.assertEqual(payload["input"], ["query"])
            self.assertEqual(payload["embed_type"], "query")


if __name__ == "__main__":
    unittest.main()
