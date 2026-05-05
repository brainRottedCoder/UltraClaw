#!/usr/bin/env python3
# ============================================================================
# ULTRACLAW — embedding_server.py
# ============================================================================
# Embedding generation service using sentence-transformers.
# Uses all-MiniLM-L6-v2 (384 dimensions, ~90MB quantized) for fast
# high-quality embeddings. Multi-language support included.
#
# Usage:
#   python embedding_server.py [--model "all-MiniLM-L6-v2"] [--batch-size 32]
#
# Commands (JSON lines on stdin):
#   {"action": "embed", "text": "Hello world"}
#   {"action": "embed_batch", "texts": ["Hello", "World"]}
#   {"action": "stats"}
#   {"action": "quit"}
#
# Responses:
#   {"status": "ok", "embedding": [0.1, -0.2, ...], "dim": 384, "model": "..."}
#   {"status": "ok", "embeddings": [[...], [...]], "count": 2}
#   {"status": "error", "message": "..."}
# ============================================================================

import sys
import json
import os
import time
from typing import List, Optional

try:
    from sentence_transformers import SentenceTransformer
    HAS_TRANSFORMERS = True
except ImportError:
    HAS_TRANSFORMERS = False


class EmbeddingServer:
    def __init__(self, model_name: str = "all-MiniLM-L6-v2", batch_size: int = 32):
        self.model_name = model_name
        self.batch_size = batch_size
        self.model = None
        self.load_time = None
        self.embed_count = 0
        self.total_embed_time = 0.0
        self._load_model()

    def _load_model(self):
        """Load the embedding model."""
        if not HAS_TRANSFORMERS:
            raise RuntimeError(
                "sentence-transformers not installed. Run: pip install sentence-transformers"
            )

        print(f"Loading embedding model: {self.model_name}", file=sys.stderr)
        start = time.time()
        try:
            self.model = SentenceTransformer(self.model_name)
            self.load_time = time.time() - start
            print(f"Model loaded in {self.load_time:.2f}s", file=sys.stderr)
            print(f"Embedding dimension: {self.model.get_sentence_embedding_dimension()}", file=sys.stderr)
            print(f"Pretty name: {self.model_name}", file=sys.stderr)
        except Exception as e:
            raise RuntimeError(f"Failed to load model '{self.model_name}': {e}")

    def embed(self, text: str) -> Optional[List[float]]:
        """Generate embedding for a single text."""
        if not text:
            return None

        try:
            start = time.time()
            embedding = self.model.encode(text, normalize_embeddings=True)
            elapsed = time.time() - start

            self.embed_count += 1
            self.total_embed_time += elapsed

            return embedding.tolist()
        except Exception as e:
            print(f"Embedding error: {e}", file=sys.stderr)
            return None

    def embed_batch(self, texts: List[str]) -> List[Optional[List[float]]]:
        """Generate embeddings for a batch of texts."""
        if not texts:
            return []

        # Filter out empty texts
        valid_texts = [t if t else " " for t in texts]

        try:
            start = time.time()
            embeddings = self.model.encode(valid_texts, batch_size=self.batch_size, normalize_embeddings=True)
            elapsed = time.time() - start

            self.embed_count += len(texts)
            self.total_embed_time += elapsed

            results = []
            for i, emb in enumerate(embeddings):
                if texts[i]:  # Original was non-empty
                    results.append(emb.tolist())
                else:
                    results.append(None)

            return results
        except Exception as e:
            print(f"Batch embedding error: {e}", file=sys.stderr)
            return [None] * len(texts)

    def stats(self) -> dict:
        """Return server statistics."""
        avg_time = self.total_embed_time / self.embed_count if self.embed_count > 0 else 0
        return {
            "model": self.model_name,
            "dimension": self.model.get_sentence_embedding_dimension(),
            "load_time_s": round(self.load_time, 2) if self.load_time else 0,
            "total_embeddings": self.embed_count,
            "total_time_s": round(self.total_embed_time, 3),
            "avg_time_ms": round(avg_time * 1000, 2),
        }


def main():
    # Parse optional command line args
    model_name = "all-MiniLM-L6-v2"
    batch_size = 32

    args = sys.argv[1:]
    i = 0
    while i < len(args):
        if args[i] == "--model" and i + 1 < len(args):
            model_name = args[i + 1]
            i += 2
        elif args[i] == "--batch-size" and i + 1 < len(args):
            batch_size = int(args[i + 1])
            i += 2
        else:
            i += 1

    # Initialize server
    try:
        server = EmbeddingServer(model_name, batch_size)
    except Exception as e:
        print(json.dumps({"status": "error", "message": str(e)}), file=sys.stdout)
        sys.stdout.flush()
        sys.exit(1)

    print(f"Embedding server ready (model: {model_name}, batch_size: {batch_size})", file=sys.stderr)
    sys.stderr.flush()

    # Main loop
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue

        try:
            cmd = json.loads(line)
        except json.JSONDecodeError:
            print(json.dumps({"status": "error", "message": "Invalid JSON"}))
            sys.stdout.flush()
            continue

        action = cmd.get("action", "")

        if action == "quit":
            print(json.dumps({"status": "ok", "message": "Shutting down", "stats": server.stats()}))
            sys.stdout.flush()
            break

        elif action == "embed":
            text = cmd.get("text", "")
            embedding = server.embed(text)
            if embedding is not None:
                print(json.dumps({
                    "status": "ok",
                    "embedding": embedding,
                    "dim": len(embedding),
                    "model": model_name
                }))
            else:
                print(json.dumps({"status": "error", "message": "Embedding generation failed"}))

        elif action == "embed_batch":
            texts = cmd.get("texts", [])
            if not isinstance(texts, list):
                print(json.dumps({"status": "error", "message": "'texts' must be a list"}))
                sys.stdout.flush()
                continue

            embeddings = server.embed_batch(texts)
            valid_count = sum(1 for e in embeddings if e is not None)
            print(json.dumps({
                "status": "ok",
                "embeddings": embeddings,
                "count": valid_count,
                "total": len(texts),
                "dim": len(embeddings[0]) if embeddings and embeddings[0] else 0,
                "model": model_name
            }))

        elif action == "stats":
            print(json.dumps({"status": "ok", "stats": server.stats()}))

        else:
            print(json.dumps({"status": "error", "message": f"Unknown action: {action}"}))

        sys.stdout.flush()


if __name__ == "__main__":
    main()