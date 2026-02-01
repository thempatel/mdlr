//! Ignore storage operations for CacheStore.
//!
//! This module extends CacheStore with methods for managing per-unit metric
//! ignores. Ignores allow users to suppress specific metrics for specific
//! symbols to reduce false positives in check output.

use super::store::CacheStore;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;

const IGNORES_FILE: &str = "ignores.json";

/// Storage for per-unit metric ignores.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Ignores {
    pub version: u32,
    /// Maps symbol ID to list of ignored metric names
    pub ignores: HashMap<String, Vec<String>>,
}

impl Ignores {
    pub fn new() -> Self {
        Self { version: 1, ignores: HashMap::new() }
    }

    /// Add an ignore for a metric on a symbol.
    pub fn add(&mut self, symbol: &str, metric: &str) {
        let entry = self.ignores.entry(symbol.to_string()).or_default();
        if !entry.contains(&metric.to_string()) {
            entry.push(metric.to_string());
        }
    }

    /// Remove an ignore for a metric on a symbol.
    /// Returns true if the ignore was found and removed.
    pub fn remove(&mut self, symbol: &str, metric: &str) -> bool {
        if let Some(entry) = self.ignores.get_mut(symbol) {
            if let Some(pos) = entry.iter().position(|m| m == metric) {
                entry.remove(pos);
                if entry.is_empty() {
                    self.ignores.remove(symbol);
                }
                return true;
            }
        }
        false
    }

    /// Check if a metric is ignored for a symbol.
    pub fn is_ignored(&self, symbol: &str, metric: &str) -> bool {
        self.ignores
            .get(symbol)
            .map(|v| v.contains(&metric.to_string()))
            .unwrap_or(false)
    }

    /// Check if the ignores are empty.
    pub fn is_empty(&self) -> bool {
        self.ignores.is_empty()
    }
}

/// Ignore storage operations for CacheStore.
impl CacheStore {
    /// Get the path to the ignores file.
    fn ignores_path(&self) -> std::path::PathBuf {
        self.root().join(".mdlr").join(IGNORES_FILE)
    }

    /// Load ignores from storage.
    pub fn load_ignores(&self) -> Result<Ignores> {
        let path = self.ignores_path();
        if !path.exists() {
            return Ok(Ignores::new());
        }

        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read ignores: {:?}", path))?;
        let ignores: Ignores = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse ignores: {:?}", path))?;
        Ok(ignores)
    }

    /// Save ignores to storage.
    pub fn save_ignores(&self, ignores: &Ignores) -> Result<()> {
        let path = self.ignores_path();

        if ignores.is_empty() {
            // Remove the file if there are no ignores
            if path.exists() {
                fs::remove_file(&path).with_context(|| {
                    format!("Failed to remove ignores: {:?}", path)
                })?;
            }
            return Ok(());
        }

        let content = serde_json::to_string_pretty(ignores)?;
        fs::write(&path, content)
            .with_context(|| format!("Failed to write ignores: {:?}", path))?;
        Ok(())
    }
}
