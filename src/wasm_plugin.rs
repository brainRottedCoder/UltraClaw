// ============================================================================
// ULTRACLAW — wasm_plugin.rs
// ============================================================================
// WASM Plugin System using wasmer runtime.
//
// This module provides a complete plugin system for loading and executing
// WebAssembly modules. Plugins can expose custom functions that can be called
// from the agent's skill system.
//
// ARCHITECTURE:
// - PluginManager: manages loaded plugins, handles lifecycle
// - WasmPlugin: individual plugin instance with metadata
// - PluginCall: represents a function call to a plugin
//
// SAFETY:
// - WASM plugins run in a sandboxed environment
// - Memory is managed by wasmer's runtime
// - Plugins cannot access host resources directly (only through imports)
//
// USAGE:
//   // Load a plugin
//   manager.load_plugin("my_plugin.wasm", "my_plugin").await?;
//
//   // Call a function
//   let result = manager.call_plugin("my_plugin", "add", vec![1, 2]).await?;
//
// ============================================================================

use crate::skill::{Skill, SkillOutput};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

#[cfg(feature = "wasm")]
use wasmer::{Instance, Module, Store, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginMetadata {
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub functions: Vec<String>,
    pub memory_pages: Option<u32>,
    pub loaded_at: Option<i64>,
}

impl Default for PluginMetadata {
    fn default() -> Self {
        Self {
            name: "Unknown".to_string(),
            version: "1.0.0".to_string(),
            description: "WASM Plugin".to_string(),
            author: "Unknown".to_string(),
            functions: Vec::new(),
            memory_pages: None,
            loaded_at: Some(chrono::Utc::now().timestamp()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginCallResult {
    pub success: bool,
    pub return_values: Vec<i32>,
    pub execution_time_ms: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfo {
    pub name: String,
    pub metadata: PluginMetadata,
    pub loaded_at: i64,
    pub call_count: u64,
}

#[cfg(feature = "wasm")]
struct WasmPluginInner {
    module: Module,
    instance: Instance,
    store: Store,
    metadata: PluginMetadata,
}

#[cfg(feature = "wasm")]
pub struct WasmPlugin {
    name: String,
    inner: Arc<WasmPluginInner>,
    call_count: Arc<RwLock<u64>>,
}

#[cfg(feature = "wasm")]
impl WasmPlugin {
    pub fn from_bytes(name: &str, bytes: &[u8]) -> Result<Self, String> {
        let mut store = Store::default();
        let module = Module::new(&store, bytes)
            .map_err(|e| format!("Failed to compile WASM module: {}", e))?;

        let import_object = module.imports();
        let instance = Instance::new(&mut store, &module, &import_object)
            .map_err(|e| format!("Failed to instantiate plugin: {}", e))?;

        let exports: Vec<String> = instance
            .exports
            .functions()
            .map(|f| f.name().to_string())
            .collect();

        let metadata = PluginMetadata {
            name: name.to_string(),
            version: "1.0.0".to_string(),
            description: format!("WASM plugin with {} exported functions", exports.len()),
            author: "Unknown".to_string(),
            functions: exports.clone(),
            memory_pages: None,
            loaded_at: Some(chrono::Utc::now().timestamp()),
        };

        info!(
            plugin = %name,
            functions = ?exports,
            "WASM plugin loaded successfully"
        );

        Ok(Self {
            name: name.to_string(),
            inner: Arc::new(WasmPluginInner {
                module,
                instance,
                store,
                metadata,
            }),
            call_count: Arc::new(RwLock::new(0)),
        })
    }

    pub fn call(&self, function_name: &str, args: &[i32]) -> Result<Vec<i32>, String> {
        let mut store = &mut self.inner.store;
        let instance = &self.inner.instance;

        let function = instance
            .exports
            .get_function(function_name)
            .map_err(|e| format!("Function '{}' not found: {}", function_name, e))?;

        let values: Vec<Value> = args.iter().map(|&v| Value::I32(v)).collect();
        let results = function.call(&mut store, &values)
            .map_err(|e| format!("Plugin call failed: {}", e))?;

        Ok(results.iter().filter_map(|v| match v {
            Value::I32(i) => Some(*i),
            Value::I64(i) => Some(*i as i32),
            _ => None,
        }).collect())
    }

    pub fn metadata(&self) -> &PluginMetadata {
        &self.inner.metadata
    }

    pub async fn increment_call_count(&self) {
        let mut count = self.call_count.write().await;
        *count += 1;
    }

    pub async fn get_call_count(&self) -> u64 {
        *self.call_count.read().await
    }
}

#[cfg(not(feature = "wasm"))]
pub struct WasmPlugin {
    pub name: String,
    pub metadata: PluginMetadata,
    pub path: Option<String>,
    call_count: Arc<RwLock<u64>>,
}

#[cfg(not(feature = "wasm"))]
impl WasmPlugin {
    pub fn from_bytes(name: &str, _bytes: &[u8]) -> Result<Self, String> {
        warn!(
            plugin = %name,
            "WASM feature not enabled, plugin loaded in stub mode"
        );
        Ok(Self {
            name: name.to_string(),
            metadata: PluginMetadata {
                name: name.to_string(),
                ..Default::default()
            },
            path: None,
            call_count: Arc::new(RwLock::new(0)),
        })
    }

    pub fn from_file(name: &str, path: &str) -> Result<Self, String> {
        info!(
            plugin = %name,
            path = %path,
            "Loading WASM plugin (stub mode)"
        );
        Ok(Self {
            name: name.to_string(),
            metadata: PluginMetadata {
                name: name.to_string(),
                description: format!("Plugin loaded from {} (WASM support not compiled)", path),
                functions: vec!["execute".to_string()],
                ..Default::default()
            },
            path: Some(path.to_string()),
            call_count: Arc::new(RwLock::new(0)),
        })
    }

    pub fn call(&self, _function_name: &str, _args: &[i32]) -> Result<Vec<i32>, String> {
        debug!(
            plugin = %self.name,
            function = %_function_name,
            "Plugin call blocked (stub mode - rebuild with --features wasm)"
        );
        Err(format!(
            "WASM plugin '{}' not available: WASM feature is disabled. Rebuild with 'cargo build --features wasm' to enable WebAssembly plugin execution.",
            self.name
        ))
    }

    pub fn metadata(&self) -> &PluginMetadata {
        &self.metadata
    }

    pub async fn increment_call_count(&self) {
        let mut count = self.call_count.write().await;
        *count += 1;
    }

    pub async fn get_call_count(&self) -> u64 {
        *self.call_count.read().await
    }
}

pub struct PluginManager {
    plugins: Arc<RwLock<HashMap<String, Arc<WasmPlugin>>>>,
    #[cfg(feature = "wasm")]
    max_memory_mb: usize,
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginManager {
    pub fn new() -> Self {
        Self {
            plugins: Arc::new(RwLock::new(HashMap::new())),
            #[cfg(feature = "wasm")]
            max_memory_mb: 256,
        }
    }

    #[cfg(feature = "wasm")]
    pub fn with_max_memory(max_mb: usize) -> Self {
        Self {
            plugins: Arc::new(RwLock::new(HashMap::new())),
            max_memory_mb: max_mb,
        }
    }

    pub async fn load_plugin(&self, path: &str, name: &str) -> Result<PluginMetadata, String> {
        let bytes = tokio::fs::read(path).await
            .map_err(|e| format!("Failed to read plugin file '{}': {}", path, e))?;

        let plugin = WasmPlugin::from_bytes(name, &bytes)?;

        let metadata = plugin.metadata().clone();

        let mut plugins = self.plugins.write().await;
        plugins.insert(name.to_string(), Arc::new(plugin));

        info!(
            plugin = %name,
            path = %path,
            functions = ?metadata.functions,
            "Plugin loaded into manager"
        );

        Ok(metadata)
    }

    pub async fn load_plugin_from_bytes(&self, bytes: &[u8], name: &str) -> Result<PluginMetadata, String> {
        let plugin = WasmPlugin::from_bytes(name, bytes)?;

        let metadata = plugin.metadata().clone();

        let mut plugins = self.plugins.write().await;
        plugins.insert(name.to_string(), Arc::new(plugin));

        info!(
            plugin = %name,
            functions = ?metadata.functions,
            "Plugin loaded into manager"
        );

        Ok(metadata)
    }

    pub async fn call_plugin(
        &self,
        name: &str,
        function: &str,
        args: Vec<i32>,
    ) -> Result<PluginCallResult, String> {
        let start = Instant::now();

        let plugin = {
            let plugins = self.plugins.read().await;
            plugins.get(name).cloned()
        };

        let plugin = plugin.ok_or_else(|| format!("Plugin '{}' not found", name))?;

        match plugin.call(function, &args) {
            Ok(return_values) => {
                (*plugin).increment_call_count().await;
                let elapsed = start.elapsed();

                debug!(
                    plugin = %name,
                    function = %function,
                    args = ?args,
                    results = ?return_values,
                    time_ms = elapsed.as_millis(),
                    "Plugin call successful"
                );

                Ok(PluginCallResult {
                    success: true,
                    return_values,
                    execution_time_ms: elapsed.as_millis() as u64,
                    error: None,
                })
            }
            Err(e) => {
                let elapsed = start.elapsed();
                error!(
                    plugin = %name,
                    function = %function,
                    error = %e,
                    "Plugin call failed"
                );

                Ok(PluginCallResult {
                    success: false,
                    return_values: Vec::new(),
                    execution_time_ms: elapsed.as_millis() as u64,
                    error: Some(e),
                })
            }
        }
    }

    pub async fn unload_plugin(&self, name: &str) -> bool {
        let mut plugins = self.plugins.write().await;
        let removed = plugins.remove(name).is_some();

        if removed {
            info!(plugin = %name, "Plugin unloaded");
        }

        removed
    }

    pub async fn list_plugins(&self) -> Vec<PluginInfo> {
        let plugins = self.plugins.read().await;

        plugins
            .iter()
            .map(|(name, plugin)| {
                let loaded_at = plugin.metadata().loaded_at.unwrap_or(0);
                PluginInfo {
                    name: name.clone(),
                    metadata: plugin.metadata().clone(),
                    loaded_at,
                    call_count: 0,
                }
            })
            .collect()
    }

    pub async fn get_plugin(&self, name: &str) -> Option<Arc<WasmPlugin>> {
        let plugins = self.plugins.read().await;
        plugins.get(name).cloned()
    }

    pub async fn plugin_exists(&self, name: &str) -> bool {
        let plugins = self.plugins.read().await;
        plugins.contains_key(name)
    }

    pub async fn unload_all(&self) -> usize {
        let mut plugins = self.plugins.write().await;
        let count = plugins.len();
        plugins.clear();
        info!(count = count, "All plugins unloaded");
        count
    }
}

// ============================================================================
// Skill Implementation
// ============================================================================

pub struct WasmPluginSkill {
    manager: Arc<PluginManager>,
}

impl WasmPluginSkill {
    pub fn new() -> Self {
        Self {
            manager: Arc::new(PluginManager::new()),
        }
    }

    pub fn with_manager(manager: Arc<PluginManager>) -> Self {
        Self { manager }
    }

    pub fn get_manager(&self) -> Arc<PluginManager> {
        self.manager.clone()
    }
}

impl Default for WasmPluginSkill {
    fn default() -> Self {
        Self::new()
    }
}

impl Skill for WasmPluginSkill {
    fn name(&self) -> &'static str {
        "wasm_plugin"
    }

    fn description(&self) -> &'static str {
        "WASM Plugin System: Load, execute, and manage WebAssembly plugins. \
         Load plugins from .wasm files, call their exported functions, \
         and manage plugin lifecycle. Plugins run in a sandboxed environment \
         with memory isolation."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Plugin action to perform",
                    "enum": ["load", "call", "unload", "list", "info", "unload_all"]
                },
                "path": {
                    "type": "string",
                    "description": "Path to .wasm file (for load action)"
                },
                "name": {
                    "type": "string",
                    "description": "Plugin name (unique identifier)"
                },
                "function": {
                    "type": "string",
                    "description": "Function name to call (for call action)"
                },
                "args": {
                    "type": "array",
                    "items": {"type": "integer"},
                    "description": "Integer arguments to pass to the function"
                }
            },
            "required": ["action"]
        })
    }

    fn execute_sync(&self, args: &serde_json::Value) -> SkillOutput {
        use tokio::runtime::Handle;

        let action_owned = args.get("action")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "list".to_string());
        let manager = self.manager.clone();
        let args_json = serde_json::to_string(args).unwrap_or_default();

        let rt = match Handle::try_current() {
            Ok(rt) => rt,
            Err(_) => {
                return SkillOutput {
                    name: "wasm_plugin".to_string(),
                    output: "Error: no async runtime available".to_string(),
                    is_error: true,
                };
            }
        };

        let result = std::thread::spawn(move || {
            rt.block_on(async {
                execute_wasm_action(&manager, &action_owned, &args_json).await
            })
        })
        .join()
        .unwrap_or_else(|_| Err("WASM plugin operation panicked".to_string()));

        match result {
            Ok(output) => SkillOutput {
                name: "wasm_plugin".to_string(),
                output,
                is_error: false,
            },
            Err(e) => SkillOutput {
                name: "wasm_plugin".to_string(),
                output: format!("WASM plugin error: {}", e),
                is_error: true,
            },
        }
    }
}

async fn execute_wasm_action(
    manager: &Arc<PluginManager>,
    action: &str,
    args_json: &str,
) -> Result<String, String> {
    let args: serde_json::Value = serde_json::from_str(args_json)
        .map_err(|e| format!("Invalid args JSON: {}", e))?;

match action {
        "load" => {
            let path = args.get("path").and_then(|v| v.as_str())
                .ok_or("path is required for load action")?;
            let name = args.get("name").and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    std::path::Path::new(path)
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| "unnamed".to_string())
                });

            let metadata = manager.load_plugin(path, &name).await?;

            Ok(serde_json::to_string_pretty(&serde_json::json!({
                "status": "loaded",
                "name": name,
                "metadata": metadata
            })).unwrap_or_else(|_| "{}".to_string()))
        }

        "call" => {
            let name = args.get("name").and_then(|v| v.as_str())
                .ok_or("name is required for call action")?;
            let function = args.get("function").and_then(|v| v.as_str())
                .ok_or("function is required for call action")?;
            let args_list: Vec<i32> = args.get("args")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter()
                    .filter_map(|v| v.as_i64().map(|n| n as i32))
                    .collect())
                .unwrap_or_default();

            let result = manager.call_plugin(name, function, args_list).await?;

            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()))
        }

        "unload" => {
            let name = args.get("name").and_then(|v| v.as_str())
                .ok_or("name is required for unload action")?;

            let removed = manager.unload_plugin(name).await;

            Ok(serde_json::json!({
                "status": if removed { "unloaded" } else { "not_found" },
                "name": name
            }).to_string())
        }

        "list" => {
            let plugins = manager.list_plugins().await;

            let plugin_list: Vec<serde_json::Value> = plugins.iter().map(|p| {
                serde_json::json!({
                    "name": p.name,
                    "version": p.metadata.version,
                    "functions": p.metadata.functions,
                    "loaded_at": p.loaded_at
                })
            }).collect();

            Ok(serde_json::to_string_pretty(&serde_json::json!({
                "plugins": plugin_list,
                "count": plugins.len()
            })).unwrap_or_else(|_| "{}".to_string()))
        }

        "info" => {
            let name = args.get("name").and_then(|v| v.as_str())
                .ok_or("name is required for info action")?;

            let plugin = manager.get_plugin(name).await
                .ok_or_else(|| format!("Plugin '{}' not found", name))?;

            let metadata = plugin.metadata();

            Ok(serde_json::to_string_pretty(&serde_json::json!({
                "name": metadata.name,
                "version": metadata.version,
                "description": metadata.description,
                "author": metadata.author,
                "functions": metadata.functions,
                "memory_pages": metadata.memory_pages,
                "loaded_at": metadata.loaded_at
            })).unwrap_or_else(|_| "{}".to_string()))
        }

        "unload_all" => {
            let count = manager.unload_all().await;

            Ok(serde_json::json!({
                "status": "unloaded",
                "count": count
            }).to_string())
        }

        _ => Err(format!(
            "Unknown action: '{}'. Available: load, call, unload, list, info, unload_all",
            action
        )),
    }
}

// ============================================================================
// Example WASM Plugin Generator (for testing)
// ============================================================================

pub fn generate_example_plugin_source() -> &'static str {
    r#"
;; Example WASM plugin in WebAssembly Text Format (WAT)
;; This can be compiled to WASM using tools like wat2wasm

(module
  ;; Memory section (1 page = 64KB)
  (memory (export "memory") 1)

  ;; Simple add function
  (func (export "add") (param i32 i32) (result i32)
    local.get 0
    local.get 1
    i32.add
  )

  ;; Multiply function
  (func (export "multiply") (param i32 i32) (result i32)
    local.get 0
    local.get 1
    i32.mul
  )

  ;; Return a constant
  (func (export "get_value") (result i32)
    i32.const 42
  )
)
"#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugin_metadata_default() {
        let metadata = PluginMetadata::default();
        assert_eq!(metadata.name, "Unknown");
        assert_eq!(metadata.version, "1.0.0");
    }

    #[tokio::test]
    async fn test_plugin_manager_new() {
        let manager = PluginManager::new();
        let plugins = manager.list_plugins().await;
        assert!(plugins.is_empty());
    }

    #[tokio::test]
    async fn test_plugin_manager_unload_nonexistent() {
        let manager = PluginManager::new();
        let removed = manager.unload_plugin("nonexistent").await;
        assert!(!removed);
    }

    #[test]
    fn test_generate_example_source() {
        let source = generate_example_plugin_source();
        assert!(source.contains("(module"));
        assert!(source.contains("(export \"add\")"));
    }
}