//! `rivet.toml`: the design description shared by the CLI runner and the
//! Verilator build helper.
//!
//! ```toml
//! [design]
//! top = "dff"
//! sources = ["hdl/dff.sv"]
//! includes = ["hdl/include"]
//! timescale = "1ns/1ps"
//!
//! [design.defines]
//! SIM = "1"
//!
//! [design.params]
//! WIDTH = "8"
//!
//! [sim.icarus]
//! args = ["-g2012"]
//!
//! [sim.verilator]
//! args = ["-Wno-WIDTH"]
//! trace = true
//! timing = false
//! ```

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub design: Design,
    #[serde(default)]
    pub sim: BTreeMap<String, SimConfig>,
    /// Directory the manifest was loaded from; paths are relative to it.
    #[serde(skip)]
    pub dir: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Design {
    pub top: String,
    #[serde(default)]
    pub sources: Vec<PathBuf>,
    #[serde(default)]
    pub includes: Vec<PathBuf>,
    #[serde(default)]
    pub defines: BTreeMap<String, String>,
    #[serde(default)]
    pub params: BTreeMap<String, String>,
    pub timescale: Option<String>,
    /// `verilog` (default) or `vhdl`.
    #[serde(default)]
    pub language: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SimConfig {
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub trace: bool,
    #[serde(default)]
    pub timing: bool,
    /// Extra runtime (plus)args for the simulator executable.
    #[serde(default)]
    pub run_args: Vec<String>,
}

impl Manifest {
    pub fn load(path: &Path) -> Result<Manifest, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let mut m: Manifest = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        m.dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        Ok(m)
    }

    /// Find `rivet.toml` in `dir` or its ancestors.
    pub fn find(start: &Path) -> Result<Manifest, String> {
        let mut d = Some(start);
        while let Some(dir) = d {
            let p = dir.join("rivet.toml");
            if p.exists() {
                return Manifest::load(&p);
            }
            d = dir.parent();
        }
        Err(format!("no rivet.toml found in {} or its parents", start.display()))
    }

    pub fn sources_abs(&self) -> Vec<PathBuf> {
        self.design.sources.iter().map(|s| self.dir.join(s)).collect()
    }

    pub fn includes_abs(&self) -> Vec<PathBuf> {
        self.design.includes.iter().map(|s| self.dir.join(s)).collect()
    }

    pub fn sim(&self, name: &str) -> SimConfig {
        self.sim.get(name).cloned().unwrap_or_default()
    }
}
