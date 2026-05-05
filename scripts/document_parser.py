#!/usr/bin/env python3
# ============================================================================
# ULTRACLAW — document_parser.py
# ============================================================================
# Document parsing service using pdfplumber and pandoc.
# Receives commands via JSON lines on stdin, sends responses to stdout.
#
# Usage:
#   python document_parser.py
#
# Commands:
#   {"action": "parse", "path": "/path/to/file.pdf"}
#   {"action": "extract_tables", "path": "/path/to/file.pdf"}
#   {"action": "chunk", "text": "...", "chunk_size": 1000, "overlap": 200}
#   {"action": "get_metadata", "path": "/path/to/file.pdf"}
#   {"action": "quit"}
#
# Responses:
#   {"status": "ok", "result": {...}}
#   {"status": "error", "message": "..."}
# ============================================================================

import sys
import json
import os
from pathlib import Path

try:
    import pdfplumber
    import pandoc
    HAS_PDFPLUMBER = True
except ImportError:
    HAS_PDFPLUMBER = False

try:
    from pypandoc import convert_file
    HAS_PYPANDOC = True
except ImportError:
    HAS_PYPANDOC = False


def parse_pdf(path: str) -> dict:
    """Extract text, tables, and metadata from a PDF file."""
    if not HAS_PDFPLUMBER:
        return {"error": "pdfplumber not installed. Run: pip install pdfplumber"}

    if not os.path.exists(path):
        return {"error": f"File not found: {path}"}

    try:
        with pdfplumber.open(path) as pdf:
            pages = []
            tables = []

            for i, page in enumerate(pdf.pages):
                page_data = {
                    "page_num": i + 1,
                    "width": page.width,
                    "height": page.height,
                    "text": page.extract_text() or "",
                }

                # Extract tables from this page
                page_tables = page.extract_tables()
                if page_tables:
                    for t_idx, table in enumerate(page_tables):
                        if table:
                            clean_table = []
                            for row in table:
                                if any(cell for cell in row):
                                    clean_table.append([str(cell or "").strip() for cell in row])
                            if clean_table:
                                tables.append({
                                    "page": i + 1,
                                    "table_idx": t_idx,
                                    "rows": len(clean_table),
                                    "data": clean_table
                                })

                pages.append(page_data)

            return {
                "file_type": "pdf",
                "num_pages": len(pages),
                "pages": pages,
                "tables": tables,
                "metadata": {
                    "title": pdf.metadata.get("Title", ""),
                    "author": pdf.metadata.get("Author", ""),
                    "subject": pdf.metadata.get("Subject", ""),
                }
            }
    except Exception as e:
        return {"error": f"PDF parsing failed: {str(e)}"}


def parse_docx(path: str) -> dict:
    """Extract text from DOCX using pandoc."""
    if not HAS_PYPANDOC:
        try:
            import subprocess
            result = subprocess.run(
                ["pandoc", "--from", "docx", "--to", "plain", path, "-o", "-"],
                capture_output=True, text=True
            )
            if result.returncode == 0:
                return {
                    "file_type": "docx",
                    "text": result.stdout,
                    "num_chars": len(result.stdout),
                    "tables": []
                }
            else:
                return {"error": f"pandoc failed: {result.stderr}"}
        except FileNotFoundError:
            return {"error": "Neither pypandoc nor pandoc is available"}
    else:
        try:
            text = convert_file(path, "plain")
            return {
                "file_type": "docx",
                "text": text,
                "num_chars": len(text),
                "tables": []
            }
        except Exception as e:
            return {"error": f"DOCX parsing failed: {str(e)}"}


def parse_txt(path: str) -> dict:
    """Read a plain text file."""
    try:
        with open(path, "r", encoding="utf-8", errors="replace") as f:
            text = f.read()
        return {
            "file_type": "txt",
            "text": text,
            "num_chars": len(text),
            "num_lines": text.count("\n") + 1
        }
    except Exception as e:
        return {"error": f"Text file read failed: {str(e)}"}


def parse_document(path: str) -> dict:
    """Auto-detect file type and parse accordingly."""
    path_lower = path.lower()
    if path_lower.endswith(".pdf"):
        return parse_pdf(path)
    elif path_lower.endswith((".docx", ".doc")):
        return parse_docx(path)
    elif path_lower.endswith(".txt"):
        return parse_txt(path)
    elif path_lower.endswith((".md", ".rst", ".html")):
        return parse_txt(path)
    else:
        return {"error": f"Unsupported file type: {path}"}


def chunk_text(text: str, chunk_size: int = 1000, overlap: int = 200) -> list:
    """Split text into overlapping chunks for RAG processing."""
    if not text or chunk_size <= 0:
        return []

    chunks = []
    start = 0
    text_len = len(text)

    while start < text_len:
        end = start + chunk_size
        chunk = text[start:end]

        # Try to break at sentence boundary
        if end < text_len:
            last_period = chunk.rfind(". ")
            last_newline = chunk.rfind("\n")
            last_break = max(last_period, last_newline)

            if last_break > chunk_size // 2:
                end = start + last_break + 1
                chunk = text[start:end]

        chunks.append({
            "text": chunk.strip(),
            "start": start,
            "end": end,
            "length": end - start
        })

        # Move forward with overlap
        start = end - overlap
        if start >= text_len - chunk_size:
            start = text_len - chunk_size
        if start <= 0:
            break

    return chunks


def get_metadata(path: str) -> dict:
    """Get basic file metadata."""
    if not os.path.exists(path):
        return {"error": f"File not found: {path}"}

    stat = os.stat(path)
    return {
        "path": path,
        "size_bytes": stat.st_size,
        "modified": stat.st_mtime,
        "name": os.path.basename(path),
        "extension": Path(path).suffix.lower()
    }


def main():
    """Main loop: read JSON commands from stdin, write JSON responses to stdout."""
    print("Document parser ready", file=sys.stderr)
    sys.stderr.flush()

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
            print(json.dumps({"status": "ok", "message": "Shutting down"}))
            sys.stdout.flush()
            break

        elif action == "parse":
            path = cmd.get("path", "")
            result = parse_document(path)
            print(json.dumps({"status": "ok", "result": result}))

        elif action == "extract_tables":
            path = cmd.get("path", "")
            if not path.lower().endswith(".pdf"):
                print(json.dumps({"status": "error", "message": "Table extraction only supported for PDF files"}))
                sys.stdout.flush()
                continue

            result = parse_pdf(path)
            if "tables" in result:
                print(json.dumps({"status": "ok", "tables": result["tables"]}))
            else:
                print(json.dumps({"status": "ok", "tables": []}))

        elif action == "chunk":
            text = cmd.get("text", "")
            chunk_size = cmd.get("chunk_size", 1000)
            overlap = cmd.get("overlap", 200)
            chunks = chunk_text(text, chunk_size, overlap)
            print(json.dumps({"status": "ok", "chunks": chunks, "count": len(chunks)}))

        elif action == "get_metadata":
            path = cmd.get("path", "")
            meta = get_metadata(path)
            if "error" in meta:
                print(json.dumps({"status": "error", "message": meta["error"]}))
            else:
                print(json.dumps({"status": "ok", "metadata": meta}))

        else:
            print(json.dumps({"status": "error", "message": f"Unknown action: {action}"}))

        sys.stdout.flush()


if __name__ == "__main__":
    main()