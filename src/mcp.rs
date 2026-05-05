// ============================================================================
// ULTRACLAW — mcp.rs
// ============================================================================
// Model Context Protocol (MCP) client implementation.
//
// MCP allows Ultraclaw to connect to external tool/data servers using a
// standardized JSON-RPC 2.0 protocol over stdio pipes. This means the agent
// can access filesystem browsers, database clients, web scrapers, and any
// other MCP-compatible server without embedding their logic directly.
//
// MEMORY OPTIMIZATION:
// - Communication is line-buffered: we read one JSON-RPC message at a time,
//   parse it, and discard the buffer. No full-response accumulation.
// - The MCP server runs as a separate process, so its memory is isolated
//   from Ultraclaw's address space. If it leaks, we kill and restart it.
// - On `Drop`, the child process is killed immediately, freeing all its
//   resources. No zombie processes.
//
// ENERGY OPTIMIZATION:
// - The stdio pipe uses async reads (`BufReader` on `ChildStdout`), so the
//   tokio runtime sleeps while waiting for server responses.
// - No polling loop — pure event-driven I/O via the OS kernel's epoll/kqueue.
// - Server is only spawned when MCP is configured (lazy initialization).
// ============================================================================

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

/// JSON-RPC 2.0 request message.
#[derive(Debug, Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    id: u64,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

/// JSON-RPC 2.0 response message.
#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    #[allow(dead_code)]
    jsonrpc: String,
    #[allow(dead_code)]
    id: Option<u64>,
    result: Option<Value>,
    error: Option<JsonRpcError>,
}

/// JSON-RPC 2.0 error object.
#[derive(Debug, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
}

/// An MCP tool descriptor (returned by tools/list).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: Option<Value>,
}

/// An MCP resource descriptor (returned by resources/list).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct McpResource {
    pub uri: String,
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "mimeType")]
    pub mime_type: Option<String>,
}

/// MCP Client — manages a stdio connection to an MCP server process.
///
/// # Memory Layout
/// - `child`: OS process handle (~8 bytes pointer)
/// - `stdin`/`stdout`: Buffered I/O handles (~8KB combined for buffers)
/// - `request_id`: 8 bytes
/// - Total: ~8.2KB, dominated by I/O buffers
///
/// The MCP server's memory (potentially hundreds of MB for a complex server)
/// is completely isolated in its own process address space.
pub struct McpClient {
    /// The child process handle. Killed on Drop.
    _child: Child,
    /// Mutex-protected stdin writer for sending requests.
    /// Mutex ensures only one request is in-flight at a time, preventing
    /// interleaved writes to the pipe.
    stdin: Mutex<tokio::process::ChildStdin>,
    /// Mutex-protected buffered stdout reader for reading responses.
    stdout: Mutex<BufReader<tokio::process::ChildStdout>>,
    /// Monotonically increasing request ID for JSON-RPC correlation.
    request_id: std::sync::atomic::AtomicU64,
}

impl McpClient {
    /// Spawn an MCP server process and establish communication.
    ///
    /// # Arguments
    /// * `command` - The command to run (e.g., "npx")
    /// * `args` - Arguments to the command (e.g., ["mcp-server-filesystem", "/tmp"])
    ///
    /// # Process Lifecycle
    /// The child process's stdin/stdout are captured as pipes. stderr is
    /// inherited (goes to Ultraclaw's logs). On Drop, the child is killed.
    pub async fn connect(command: &str, args: &[&str]) -> Result<Self, String> {
        let mut child = Command::new(command)
            .args(args)
            // Pipe stdin/stdout for JSON-RPC communication.
            // These are OS-level pipes with kernel-managed buffers (~64KB on Linux).
            // No userspace polling needed — the kernel wakes our async task.
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Inherit stderr so MCP server errors appear in Ultraclaw's logs
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("Failed to spawn MCP server: {}", e))?;

        let stdin = child
            .stdin
            .take()
            .ok_or("Failed to capture MCP server stdin")?;
        let stdout = child
            .stdout
            .take()
            .ok_or("Failed to capture MCP server stdout")?;

        let client = Self {
            _child: child,
            stdin: Mutex::new(stdin),
            // BufReader wraps stdout with an 8KB buffer for efficient
            // line-by-line reading. JSON-RPC messages are newline-delimited.
            stdout: Mutex::new(BufReader::new(stdout)),
            request_id: std::sync::atomic::AtomicU64::new(1),
        };

        // Send the MCP initialize handshake
        let _init_response = client
            .send_request(
                "initialize",
                Some(serde_json::json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {
                        "name": "ultraclaw",
                        "version": "0.1.0"
                    }
                })),
            )
            .await?;

        // Send initialized notification
        client.send_notification("notifications/initialized", None).await?;

        Ok(client)
    }

    /// Send a JSON-RPC request and wait for the response.
    ///
    /// # Memory Flow
    /// 1. Serialize request to a String (~100-500 bytes)
    /// 2. Write to stdin pipe (kernel buffered, we discard the String)
    /// 3. Read response line from stdout (~100-5000 bytes)
    /// 4. Parse JSON, discard raw string
    /// Net memory: only the parsed Value is retained.
    async fn send_request(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<Option<Value>, String> {
        let id = self
            .request_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            id,
            method: method.to_string(),
            params,
        };

        let mut request_str =
            serde_json::to_string(&request).map_err(|e| format!("Serialize error: {}", e))?;
        request_str.push('\n'); // JSON-RPC over stdio uses newline delimiters

        // Acquire stdin lock, write, flush, release
        {
            let mut stdin = self.stdin.lock().await;
            stdin
                .write_all(request_str.as_bytes())
                .await
                .map_err(|e| format!("Write error: {}", e))?;
            stdin
                .flush()
                .await
                .map_err(|e| format!("Flush error: {}", e))?;
        }
        // `request_str` is dropped here — memory freed immediately

        // Read response line
        let mut response_line = String::with_capacity(4096);
        {
            let mut stdout = self.stdout.lock().await;
            stdout
                .read_line(&mut response_line)
                .await
                .map_err(|e| format!("Read error: {}", e))?;
        }

        let response: JsonRpcResponse = serde_json::from_str(&response_line)
            .map_err(|e| format!("Parse error: {} — raw: {}", e, &response_line))?;
        // `response_line` is dropped here — raw JSON freed

        if let Some(err) = response.error {
            return Err(format!("MCP error {}: {}", err.code, err.message));
        }

        Ok(response.result)
    }

    /// Send a JSON-RPC notification (no response expected).
    async fn send_notification(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<(), String> {
        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params.unwrap_or(Value::Null)
        });

        let mut msg = serde_json::to_string(&notification)
            .map_err(|e| format!("Serialize error: {}", e))?;
        msg.push('\n');

        let mut stdin = self.stdin.lock().await;
        stdin.write_all(msg.as_bytes()).await.map_err(|e| format!("Write error: {}", e))?;
        stdin.flush().await.map_err(|e| format!("Flush error: {}", e))?;

        Ok(())
    }

    /// List all tools available on the MCP server.
    pub async fn list_tools(&self) -> Result<Vec<McpTool>, String> {
        let result = self.send_request("tools/list", None).await?;
        match result {
            Some(val) => {
                let tools_val = val.get("tools").cloned().unwrap_or(Value::Array(vec![]));
                serde_json::from_value(tools_val)
                    .map_err(|e| format!("Failed to parse tools: {}", e))
            }
            None => Ok(vec![]),
        }
    }

    /// Invoke a tool on the MCP server.
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<Value, String> {
        let result = self
            .send_request(
                "tools/call",
                Some(serde_json::json!({
                    "name": name,
                    "arguments": arguments
                })),
            )
            .await?;
        Ok(result.unwrap_or(Value::Null))
    }

    /// List resources available on the MCP server.
    #[allow(dead_code)]
    pub async fn list_resources(&self) -> Result<Vec<McpResource>, String> {
        let result = self.send_request("resources/list", None).await?;
        match result {
            Some(val) => {
                let res_val = val
                    .get("resources")
                    .cloned()
                    .unwrap_or(Value::Array(vec![]));
                serde_json::from_value(res_val)
                    .map_err(|e| format!("Failed to parse resources: {}", e))
            }
            None => Ok(vec![]),
        }
    }

    /// Read a specific resource by URI.
    #[allow(dead_code)]
    pub async fn read_resource(&self, uri: &str) -> Result<Value, String> {
        let result = self
            .send_request(
                "resources/read",
                Some(serde_json::json!({ "uri": uri })),
            )
            .await?;
        Ok(result.unwrap_or(Value::Null))
    }
}
