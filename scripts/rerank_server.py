#!/usr/bin/env python3
# ============================================================================
# ULTRACLAW — rerank_server.py
# ============================================================================
# Cross-encoder reranking server for advanced RAG.
#
# This server provides a REST API for re-ranking retrieved documents using
# a cross-encoder model. Cross-encoders consider both query and document
# together, providing more accurate relevance scores than bi-encoders.
#
# USAGE:
#   python scripts/rerank_server.py
#   python scripts/rerank_server.py --model cross-encoder/ms-marco-MiniLM-L-6-v2
#   RERANK_PORT=8003 python scripts/rerank_server.py
#
# ENDPOINTS:
#   POST /rerank - Rerank documents against a query
#   GET  /health - Health check
#   GET  /stats  - Server statistics
#
# MODELS:
#   - cross-encoder/ms-marco-MiniLM-L-6-v2 (default, ~100MB)
#   - cross-encoder/ms-marco-MiniLM-L-12-v2 (higher quality)
#   - cross-encoder/ms-marco-electra-base-durel-cross-encoder (latest)
#
# For more models: https://www.sbert.net/docs/pretrained-models/ce-md.html
# ============================================================================

import argparse
import json
import os
import sys
import time
from typing import Optional

try:
    from fastapi import FastAPI, HTTPException
    from pydantic import BaseModel
    from sentence_transformers import CrossEncoder
    import uvicorn
    SERVER_AVAILABLE = True
except ImportError as e:
    SERVER_AVAILABLE = False
    SERVER_IMPORT_ERROR = str(e)


app = FastAPI(
    title="Ultraclaw Rerank Server",
    description="Cross-encoder reranking for advanced RAG",
    version="1.0.0"
)


class RerankRequest(BaseModel):
    query: str
    documents: list[str]


class RerankResponse(BaseModel):
    scores: list[float]
    model: str
    count: int


class HealthResponse(BaseModel):
    status: str
    model: Optional[str] = None
    ready: bool


class StatsResponse(BaseModel):
    total_requests: int
    total_documents: int
    avg_time_ms: float
    model: str


stats = {
    "total_requests": 0,
    "total_documents": 0,
    "start_time": time.time(),
    "total_time_ms": 0.0,
}


reranker: Optional[CrossEncoder] = None
model_name: str = ""


def load_model(model: str) -> bool:
    global reranker, model_name
    try:
        print(f"Loading cross-encoder model: {model}", flush=True)
        start = time.time()
        reranker = CrossEncoder(model)
        load_time = time.time() - start
        model_name = model
        print(f"Model loaded in {load_time:.2f}s", flush=True)
        print(f"Model info: {reranker}", flush=True)
        return True
    except Exception as e:
        print(f"Failed to load model: {e}", flush=True)
        reranker = None
        return False


@app.post("/rerank", response_model=RerankResponse)
async def rerank(request: RerankRequest):
    global stats

    if reranker is None:
        raise HTTPException(
            status_code=503,
            detail="Reranker model not loaded. Check server startup logs."
        )

    if not request.documents:
        raise HTTPException(status_code=400, detail="No documents provided")

    start = time.time()

    try:
        pairs = [(request.query, doc) for doc in request.documents]
        scores = reranker.predict(pairs)

        if hasattr(scores, 'tolist'):
            scores = scores.tolist()
        elif not isinstance(scores, list):
            scores = list(scores)

        elapsed = (time.time() - start) * 1000

        stats["total_requests"] += 1
        stats["total_documents"] += len(request.documents)
        stats["total_time_ms"] += elapsed

        return RerankResponse(
            scores=scores,
            model=model_name,
            count=len(scores)
        )

    except Exception as e:
        raise HTTPException(status_code=500, detail=f"Reranking failed: {str(e)}")


@app.get("/health", response_model=HealthResponse)
async def health():
    if reranker is None:
        return HealthResponse(
            status="model_not_loaded",
            model=None,
            ready=False
        )

    return HealthResponse(
        status="ready",
        model=model_name,
        ready=True
    )


@app.get("/stats", response_model=StatsResponse)
async def get_stats():
    avg_time = (
        stats["total_time_ms"] / stats["total_requests"]
        if stats["total_requests"] > 0
        else 0.0
    )

    return StatsResponse(
        total_requests=stats["total_requests"],
        total_documents=stats["total_documents"],
        avg_time_ms=avg_time,
        model=model_name
    )


@app.get("/")
async def root():
    return {
        "service": "Ultraclaw Rerank Server",
        "version": "1.0.0",
        "endpoints": {
            "POST /rerank": "Rerank documents against a query",
            "GET /health": "Health check",
            "GET /stats": "Server statistics"
        }
    }


def main():
    if not SERVER_AVAILABLE:
        print("ERROR: Required dependencies not installed", flush=True)
        print(f"Missing: {SERVER_IMPORT_ERROR}", flush=True)
        print("\nInstall with:", flush=True)
        print("  pip install sentence-transformers fastapi uvicorn", flush=True)
        sys.exit(1)

    parser = argparse.ArgumentParser(description="Cross-encoder reranking server")
    parser.add_argument(
        "--model",
        default=os.environ.get("RERANK_MODEL", "cross-encoder/ms-marco-MiniLM-L-6-v2"),
        help="Cross-encoder model name (default: cross-encoder/ms-marco-MiniLM-L-6-v2)"
    )
    parser.add_argument(
        "--port",
        type=int,
        default=int(os.environ.get("RERANK_PORT", "8003")),
        help="Server port (default: 8003)"
    )
    parser.add_argument(
        "--host",
        default=os.environ.get("RERANK_HOST", "0.0.0.0"),
        help="Server host (default: 0.0.0.0)"
    )

    args = parser.parse_args()

    if not load_model(args.model):
        print("WARNING: Server starting without model loaded", flush=True)
        print("The /rerank endpoint will return errors until a model is available", flush=True)

    print(f"\nStarting rerank server on {args.host}:{args.port}", flush=True)
    print(f"Model: {model_name or 'not loaded'}", flush=True)
    print("Endpoints:", flush=True)
    print("  POST /rerank - Rerank documents", flush=True)
    print("  GET  /health - Health check", flush=True)
    print("  GET  /stats  - Statistics", flush=True)
    print("", flush=True)

    uvicorn.run(app, host=args.host, port=args.port, log_level="info")


if __name__ == "__main__":
    main()