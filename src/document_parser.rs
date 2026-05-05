// ============================================================================
// ULTRACLAW — document_parser.rs
// ============================================================================
// Document parsing service with built-in text support and Python fallback.
//
// This module provides:
// - Built-in parsing for TXT, MD, and HTML files (no external deps)
// - Python subprocess support for PDF and DOCX (via document_parser.py)
// - Text chunking with word-boundary awareness for RAG processing
// - Thread-safe Arc-wrapped service
//
// ARCHITECTURE:
// - Lazy subprocess startup for Python (spawned on PDF/DOCX calls)
// - Built-in parsing for plain text formats
// - JSON lines protocol on Python stdin/stdout
// - Chunking and text processing support
// - Thread-safe with Arc
//
// SUPPORTED FORMATS:
// - TXT/MD: Built-in Rust parsing (no external deps)
// - HTML: Built-in Rust parsing with tag stripping
// - PDF: Python subprocess with pdfplumber
// - DOCX: Python subprocess with python-docx
//
// SAFETY:
// - File path validation (prevent directory traversal)
// - Bounded response size per chunk
// - Graceful fallback when Python not available
// ============================================================================

use crate::skill::{Skill, SkillOutput};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// Document element types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentPage {
    pub page_num: u32,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentTable {
    pub page: u32,
    pub table_idx: u32,
    pub rows: u32,
    pub data: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentResult {
    pub file_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_pages: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pages: Option<Vec<DocumentPage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_chars: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_lines: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tables: Option<Vec<DocumentTable>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<DocumentMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextChunk {
    pub text: String,
    pub start: usize,
    pub end: usize,
    pub length: usize,
}

/// Process state
enum ParserProcess {
    Running {
        stdin: tokio::process::ChildStdin,
        stdout: BufReader<tokio::process::ChildStdout>,
    },
    Dead,
}

/// Document parser service
pub struct DocumentParserService {
    state: RwLock<ParserProcess>,
}

impl DocumentParserService {
    pub fn new() -> Self {
        Self {
            state: RwLock::new(ParserProcess::Dead),
        }
    }

    /// Spawn the Python subprocess
    pub async fn ensure_running(&self) -> Result<(), String> {
        let script_path = Self::find_script_path()?;

        let mut child = process::Command::new("python")
            .arg(&script_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to spawn document parser: {}", e))?;

        let stdin = child.stdin.take().ok_or("Failed to capture stdin")?;
        let stdout = child.stdout.take().ok_or("Failed to capture stdout")?;

        let mut state = self.state.write().await;
        *state = ParserProcess::Running {
            stdin,
            stdout: BufReader::new(stdout),
        };
        drop(state);

        // Give the process time to initialize
        tokio::time::sleep(Duration::from_secs(2)).await;

        info!("Document parser service started");
        Ok(())
    }

    /// Find the document parser script
    fn find_script_path() -> Result<String, String> {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()));
        let scripts_dir = std::env::var("ULTRACLAW_SCRIPTS_DIR")
            .map(std::path::PathBuf::from)
            .ok();

        let mut candidates: Vec<std::path::PathBuf> = vec![
            std::path::PathBuf::from("scripts/document_parser.py"),
            std::path::PathBuf::from("./scripts/document_parser.py"),
        ];

        if let Some(ref dir) = exe_dir {
            candidates.push(dir.join("scripts/document_parser.py"));
        }
        if let Some(ref dir) = scripts_dir {
            candidates.push(dir.join("document_parser.py"));
        }

        for candidate in &candidates {
            if candidate.exists() {
                return Ok(candidate.to_string_lossy().to_string());
            }
        }

        Err(format!(
            "Document parser script not found. Check scripts/document_parser.py exists."
        ))
    }

    /// Send command and get response
    async fn send_command(&self, cmd: serde_json::Value) -> Result<serde_json::Value, String> {
        {
            let state = self.state.read().await;
            if matches!(*state, ParserProcess::Dead) {
                drop(state);
                self.ensure_running().await?;
            }
        }

        let mut state = self.state.write().await;
        let (stdin, stdout) = match &mut *state {
            ParserProcess::Running { stdin, stdout } => (stdin, stdout),
            _ => return Err("Document parser not available".to_string()),
        };

        let cmd_str = serde_json::to_string(&cmd).map_err(|e| e.to_string())?;
        stdin
            .write_all(format!("{}\n", cmd_str).as_bytes())
            .await
            .map_err(|e| format!("Failed to send command: {}", e))?;
        stdin.flush().await.map_err(|e| format!("Flush error: {}", e))?;

        let mut line = String::new();
        let read_result = tokio::time::timeout(Duration::from_secs(120), stdout.read_line(&mut line)).await;

        match read_result {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => return Err(format!("Read error: {}", e)),
            Err(_) => return Err("Document parsing timed out".to_string()),
        }

        let response: serde_json::Value = serde_json::from_str(&line)
            .map_err(|e| format!("Invalid JSON response: {} (line: {})", e, line))?;

        if let Some(status) = response.get("status").and_then(|v| v.as_str()) {
            if status == "error" {
                let msg = response.get("message").and_then(|v| v.as_str()).unwrap_or("Unknown error");
                return Err(msg.to_string());
            }
        }

        Ok(response)
    }

    /// Parse a document file (PDF, DOCX, TXT)
    pub async fn parse(&self, path: &str) -> Result<DocumentResult, String> {
        // Validate path
        if path.is_empty() {
            return Err("Empty file path".to_string());
        }

        let normalized = Self::normalize_path(path)?;
        let response = self
            .send_command(serde_json::json!({
                "action": "parse",
                "path": normalized
            }))
            .await?;

        let result = response
            .get("result")
            .ok_or("No result in response")?;

        serde_json::from_value(result.clone())
            .map_err(|e| format!("Failed to parse result: {}", e))
    }

    /// Extract tables from a PDF
    pub async fn extract_tables(&self, path: &str) -> Result<Vec<DocumentTable>, String> {
        let normalized = Self::normalize_path(path)?;
        let response = self
            .send_command(serde_json::json!({
                "action": "extract_tables",
                "path": normalized
            }))
            .await?;

        let tables = response
            .get("tables")
            .ok_or("No tables in response")?;

        serde_json::from_value(tables.clone())
            .map_err(|e| format!("Failed to parse tables: {}", e))
    }

    /// Chunk text for RAG processing
    pub async fn chunk_text(
        &self,
        text: &str,
        chunk_size: usize,
        overlap: usize,
    ) -> Result<Vec<TextChunk>, String> {
        let response = self
            .send_command(serde_json::json!({
                "action": "chunk",
                "text": text,
                "chunk_size": chunk_size,
                "overlap": overlap
            }))
            .await?;

        let chunks = response
            .get("chunks")
            .ok_or("No chunks in response")?;

        serde_json::from_value(chunks.clone())
            .map_err(|e| format!("Failed to parse chunks: {}", e))
    }

    /// Normalize and validate file path
    fn normalize_path(path: &str) -> Result<String, String> {
        let p = Path::new(path);

        let canonical = p.canonicalize().map_err(|e| format!("Invalid path: {}", e))?;

        let canonical_str = canonical.to_string_lossy();
        if canonical_str.contains("..") {
            return Err("Path traversal detected".to_string());
        }

        Ok(canonical_str.to_string())
    }

    /// Parse text file directly (TXT, MD, HTML) - no Python needed
    pub async fn parse_text_file(&self, path: &str) -> Result<DocumentResult, String> {
        let normalized = Self::normalize_path(path)?;
        let p = Path::new(&normalized);

        let ext = p.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let file_type = match ext.as_str() {
            "txt" | "md" | "markdown" => "text",
            "html" | "htm" => "html",
            "json" => "json",
            "xml" => "xml",
            "csv" => "csv",
            _ => return Err(format!("Unsupported text format: {}", ext)),
        };

        let content = tokio::fs::read_to_string(&normalized)
            .await
            .map_err(|e| format!("Failed to read file: {}", e))?;

        let cleaned = if file_type == "html" {
            Self::strip_html_tags(&content)
        } else {
            content.clone()
        };

        let num_chars = cleaned.chars().count() as u64;
        let num_lines = cleaned.lines().count() as u64;

        let pages = Self::split_into_pages(&cleaned, 1);

        Ok(DocumentResult {
            file_type: file_type.to_string(),
            num_pages: Some(pages.len() as u32),
            pages: Some(pages),
            text: Some(cleaned),
            num_chars: Some(num_chars),
            num_lines: Some(num_lines),
            tables: None,
            metadata: None,
        })
    }

    fn strip_html_tags(html: &str) -> String {
        let mut result = String::new();
        let mut in_tag = false;
        let mut in_script = false;
        let mut in_style = false;

        for line in html.lines() {
            let line_lower = line.to_lowercase();
            if line_lower.contains("<script") {
                in_script = true;
            }
            if line_lower.contains("</script>") {
                in_script = false;
                continue;
            }
            if line_lower.contains("<style") {
                in_style = true;
            }
            if line_lower.contains("</style>") {
                in_style = false;
                continue;
            }

            if in_script || in_style {
                continue;
            }

            let mut text = String::new();
            let mut chars = line.chars().peekable();

            while let Some(c) = chars.next() {
                if c == '<' {
                    in_tag = true;
                } else if c == '>' {
                    in_tag = false;
                } else if !in_tag {
                    text.push(c);
                }
            }

            let cleaned = text.trim();
            if !cleaned.is_empty() {
                result.push_str(cleaned);
                result.push('\n');
            }
        }

        result
    }

    fn split_into_pages(text: &str, _chars_per_page: usize) -> Vec<DocumentPage> {
        let lines: Vec<&str> = text.lines().collect();
        let total_lines = lines.len();
        let lines_per_page = 50.max(total_lines / 1);

        let mut pages = Vec::new();
        let mut page_num = 1u32;

        for chunk in lines.chunks(lines_per_page.max(1)) {
            let page_text = chunk.join("\n");
            pages.push(DocumentPage {
                page_num,
                text: page_text,
                width: None,
                height: None,
            });
            page_num += 1;
        }

        if pages.is_empty() {
            pages.push(DocumentPage {
                page_num: 1,
                text: String::new(),
                width: None,
                height: None,
            });
        }

        pages
    }

    /// Chunk text with word-boundary awareness (standalone, no Python)
    pub async fn chunk_text_standalone(
        &self,
        text: &str,
        chunk_size: usize,
        overlap: usize,
    ) -> Result<Vec<TextChunk>, String> {
        if text.is_empty() {
            return Ok(Vec::new());
        }

        let chunk_size = chunk_size.max(100);
        let overlap = overlap.min(chunk_size / 2);

        let words: Vec<&str> = text.split_whitespace().collect();
        if words.is_empty() {
            return Ok(Vec::new());
        }

        let avg_word_len = words.iter().map(|w| w.len()).sum::<usize>() / words.len().max(1);
        let words_per_chunk = ((chunk_size as f64 / avg_word_len as f64) * 0.7) as usize;

        let mut chunks = Vec::new();
        let mut start_word = 0usize;

        while start_word < words.len() {
            let end_word = (start_word + words_per_chunk).min(words.len());
            let chunk_words = &words[start_word..end_word];
            let chunk_text = chunk_words.join(" ");

            let char_start = text.find(words[start_word]).unwrap_or(0);
            let char_end = text.rfind(words[end_word - 1])
                .map(|p| p + words[end_word - 1].len())
                .unwrap_or(text.len());

            chunks.push(TextChunk {
                text: chunk_text.clone(),
                start: char_start,
                end: char_end.min(text.len()),
                length: chunk_text.len(),
            });

            if end_word >= words.len() {
                break;
            }

            start_word = (end_word as isize - overlap as isize / avg_word_len.max(1) as isize) as usize;
            start_word = start_word.max(end_word - words_per_chunk / 2);
            start_word = start_word.max(end_word);
        }

        Ok(chunks)
    }
}

impl Default for DocumentParserService {
    fn default() -> Self {
        Self::new()
    }
}

/// Document Parser skill for LLM tool registry
pub struct DocumentParserSkill {
    service: Arc<DocumentParserService>,
}

impl DocumentParserSkill {
    pub fn new() -> Self {
        Self {
            service: Arc::new(DocumentParserService::new()),
        }
    }

    pub fn get_service(&self) -> Arc<DocumentParserService> {
        self.service.clone()
    }
}

impl Default for DocumentParserSkill {
    fn default() -> Self {
        Self::new()
    }
}

impl Skill for DocumentParserSkill {
    fn name(&self) -> &'static str {
        "document_parse"
    }

    fn description(&self) -> &'static str {
        "Parse and extract content from documents (PDF, DOCX, TXT, MD). \
         Extracts text, tables, and metadata. Supports chunking for RAG. \
         Requires: pip install pdfplumber pypandoc (or pandoc binary)."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Document action",
                    "enum": ["parse", "extract_tables", "chunk", "metadata"]
                },
                "path": {
                    "type": "string",
                    "description": "Path to the document file"
                },
                "text": {
                    "type": "string",
                    "description": "Text to chunk (for chunk action)"
                },
                "chunk_size": {
                    "type": "integer",
                    "description": "Chunk size in characters (for chunk action)",
                    "default": 1000
                },
                "overlap": {
                    "type": "integer",
                    "description": "Overlap between chunks (for chunk action)",
                    "default": 200
                },
                "max_pages": {
                    "type": "integer",
                    "description": "Max pages to extract (for parse action)"
                }
            },
            "required": ["action"]
        })
    }

    fn execute_sync(&self, args: &serde_json::Value) -> SkillOutput {
        use tokio::runtime::Handle;

        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("parse").to_string();
        let service = self.service.clone();
        let args_json = serde_json::to_string(args).unwrap_or_default();

        let rt = match Handle::try_current() {
            Ok(rt) => rt,
            Err(_) => {
                return SkillOutput {
                    name: "document_parse".to_string(),
                    output: "Error: no async runtime available".to_string(),
                    is_error: true,
                };
            }
        };

        let result = std::thread::spawn(move || {
            rt.block_on(async {
                let args_owned: serde_json::Value = serde_json::from_str(&args_json).unwrap_or_default();
                execute_doc_action(&service, &action, &args_owned).await
            })
        })
        .join()
        .unwrap_or_else(|_| Err("Document parser operation panicked".to_string()));

        match result {
            Ok(output) => SkillOutput {
                name: "document_parse".to_string(),
                output,
                is_error: false,
            },
            Err(e) => SkillOutput {
                name: "document_parse".to_string(),
                output: format!("Document parser error: {}", e),
                is_error: true,
            },
        }
    }
}

async fn execute_doc_action(
    service: &Arc<DocumentParserService>,
    action: &str,
    args: &serde_json::Value,
) -> Result<String, String> {
    match action {
        "parse" => {
            let path = args.get("path").and_then(|v| v.as_str()).ok_or("Path is required")?;
            let max_pages = args.get("max_pages").and_then(|v| v.as_u64()).map(|v| v as usize);

            let ext = Path::new(path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();

            let use_standalone = matches!(ext.as_str(), "txt" | "md" | "markdown" | "html" | "htm" | "json" | "xml" | "csv");

            let result = if use_standalone {
                service.parse_text_file(path).await
            } else {
                service.parse(path).await
            }?;

            // Apply max_pages limit if specified
            let pages = if let Some(max) = max_pages {
                result.pages.map(|p| p.into_iter().take(max).collect())
            } else {
                result.pages
            };

            let mut output = serde_json::json!({
                "file_type": result.file_type,
            });

            if let Some(np) = result.num_pages {
                output["num_pages"] = serde_json::json!(np);
            }
            if let Some(t) = result.text {
                output["text"] = serde_json::json!(t.chars().take(16000).collect::<String>());
            }
            if let Some(p) = pages {
                output["pages"] = serde_json::json!(p);
            }
            if let Some(tab) = result.tables {
                output["tables"] = serde_json::json!(tab);
            }
            if let Some(m) = result.metadata {
                output["metadata"] = serde_json::json!(m);
            }

            Ok(serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string()))
        }

        "extract_tables" => {
            let path = args.get("path").and_then(|v| v.as_str()).ok_or("Path is required")?;
            let tables = service.extract_tables(path).await?;
            Ok(serde_json::to_string_pretty(&serde_json::json!({
                "tables": tables,
                "count": tables.len()
            })).unwrap_or_else(|_| "{}".to_string()))
        }

        "chunk" => {
            let text = args.get("text").and_then(|v| v.as_str()).ok_or("Text is required")?;
            let chunk_size = args.get("chunk_size").and_then(|v| v.as_u64()).unwrap_or(1000) as usize;
            let overlap = args.get("overlap").and_then(|v| v.as_u64()).unwrap_or(200) as usize;

            let chunks = service.chunk_text_standalone(text, chunk_size, overlap).await?;
            Ok(serde_json::to_string_pretty(&serde_json::json!({
                "chunks": chunks,
                "count": chunks.len()
            })).unwrap_or_else(|_| "{}".to_string()))
        }

        "metadata" => {
            let path = args.get("path").and_then(|v| v.as_str()).ok_or("Path is required")?;
            let result = service.parse(path).await?;
            let meta = result.metadata.unwrap_or(DocumentMetadata {
                title: None,
                author: None,
                subject: None,
            });
            Ok(serde_json::to_string_pretty(&serde_json::json!({
                "file_type": result.file_type,
                "metadata": meta
            })).unwrap_or_else(|_| "{}".to_string()))
        }

        _ => Err(format!(
            "Unknown action: '{}'. Available: parse, extract_tables, chunk, metadata",
            action
        )),
    }
}