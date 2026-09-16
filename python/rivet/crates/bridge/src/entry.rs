//! Starting a Python regression inside a simulator.
//!
//! The plugin is loaded by the simulator like any other PLI module. At
//! start-of-simulation this imports the testbench modules named by
//! `RIVET_PYTHON_TESTS`, which runs their `@rivet.test` decorators, and
//! then hands the collected tests to Rivet's regression loop.

use pyo3::prelude::*;
use pyo3::types::PyList;
use rivet_core::handle::Module;
use rivet_core::runtime;
use rivet_core::test::{dump_hierarchy, finish_with, run_regression_specs};

use crate::registry;

/// Modules to import, from `RIVET_PYTHON_TESTS` (comma- or
/// colon-separated), as cocotb's `MODULE` is.
fn test_modules() -> Vec<String> {
    std::env::var("RIVET_PYTHON_TESTS")
        .unwrap_or_default()
        .split([',', ':'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Extra `sys.path` entries, from `RIVET_PYTHON_PATH`.
fn extra_paths() -> Vec<String> {
    let mut v: Vec<String> = std::env::var("RIVET_PYTHON_PATH")
        .unwrap_or_default()
        .split([',', ':'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    // The directory `rivet run` was invoked in, so a testbench beside
    // `rivet.toml` is importable without any configuration at all.
    if let Ok(cwd) = std::env::current_dir() {
        v.push(cwd.display().to_string());
    }
    v
}

/// Import the testbench modules. Returns how many tests they registered.
pub fn load_tests(py: Python<'_>) -> PyResult<usize> {
    let sys = py.import("sys")?;
    let path = sys.getattr("path")?;
    let path: &Bound<'_, PyList> = path.downcast()?;
    // Inserted back to front, so the first entry of RIVET_PYTHON_PATH
    // ends up first on `sys.path`.
    for p in extra_paths().into_iter().rev() {
        path.insert(0, p)?;
    }
    let modules = test_modules();
    if modules.is_empty() {
        log::warn!("RIVET_PYTHON_TESTS is empty: no Python testbench to import");
    }
    let before = registry::count();
    for m in &modules {
        log::debug!("importing Python testbench {m}");
        py.import(m.as_str())?;
    }
    Ok(registry::count() - before)
}

/// Install the start-of-simulation entry that runs the Python tests.
/// Called by the plugin instead of `rivet_core::test::install_default_entry`.
pub fn install() {
    runtime::set_entry(|| {
        let root_name = std::env::var("RIVET_TOPLEVEL").ok().filter(|s| !s.is_empty());
        let root = match runtime::backend(|b| b.root(root_name.as_deref())) {
            Ok(h) => h,
            Err(e) => {
                log::error!("cannot find top-level instance: {e}");
                runtime::finish();
                return;
            }
        };
        runtime::set_root(root);
        let module = Module::from_handle(root);

        if let Ok(path) = std::env::var("RIVET_DUMP_HIERARCHY") {
            match dump_hierarchy(module, std::path::Path::new(&path)) {
                Ok(()) => log::info!("wrote hierarchy to {path}"),
                Err(e) => log::error!("cannot write {path}: {e}"),
            }
            runtime::with(|rt| rt.exit_code = 0);
            runtime::finish();
            return;
        }

        // Importing the testbench happens after the hierarchy exists, so
        // a module that inspects the design at import time can.
        if let Err(e) = Python::with_gil(|py| load_tests(py).map_err(|e| crate::task::format_exception(py, &e))) {
            log::error!("cannot import the Python testbench:\n{e}");
            runtime::with(|rt| rt.exit_code = 1);
            runtime::finish();
            return;
        }

        let filter = std::env::var("RIVET_TEST_FILTER").ok();
        if let Ok(path) = std::env::var("RIVET_LIST_TESTS") {
            let names: Vec<String> =
                registry::specs().iter().filter(|s| s.is_selected(filter.as_deref())).map(|s| s.full_name()).collect();
            match std::fs::write(&path, names.join("\n") + "\n") {
                Ok(()) => log::info!("listed {} test(s) to {path}", names.len()),
                Err(e) => log::error!("cannot write {path}: {e}"),
            }
            runtime::with(|rt| rt.exit_code = 0);
            runtime::finish();
            return;
        }

        rivet_core::task::spawn_named("regression", async move {
            let specs = registry::specs();
            let filter = std::env::var("RIVET_TEST_FILTER").ok();
            let results = run_regression_specs(module, specs, filter.as_deref()).await;
            finish_with(&results);
        });
    });

    runtime::set_premature_end_handler(|| {
        log::error!("simulator ended before all tests finished (an HDL $finish or assertion, or no clock running?)");
        runtime::report_failure("simulator ended prematurely".into());
        runtime::run_to_idle();
    });
}
