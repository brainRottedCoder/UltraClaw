#!/usr/bin/env python3
"""Check Python dependencies for Ultraclaw and report status."""
import json
import sys

deps = {}

for pkg in ["playwright", "pdfplumber", "docx", "sentence_transformers"]:
    try:
        __import__(pkg)
        deps[pkg] = True
    except ImportError:
        deps[pkg] = False

try:
    import importlib
    importlib.import_module("TTS")
    deps["TTS"] = True
except ImportError:
    deps["TTS"] = False

if deps["playwright"]:
    try:
        from playwright.sync_api import sync_playwright
        p = sync_playwright()
        p.start()
        b = p.chromium.launch(headless=True)
        b.close()
        p.stop()
        deps["playwright_browsers"] = True
    except Exception:
        deps["playwright_browsers"] = False

report = {
    "python_version": sys.version,
    "deps": deps,
    "all_core": deps["playwright"] and deps["pdfplumber"] and deps["docx"],
    "all_optional": deps["TTS"],
}
print(json.dumps(report, indent=2))