//! Generate a typed Rust view of a design from the JSON hierarchy dump the
//! harness writes when `RIVET_DUMP_HIERARCHY` is set.

use serde_json::Value;
use std::collections::HashSet;
use std::fmt::Write as _;

const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn",
    "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self",
    "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while", "abstract", "become",
    "box", "do", "final", "macro", "override", "priv", "typeof", "unsized", "virtual", "yield", "try", "module",
];

pub fn is_keyword(s: &str) -> bool {
    KEYWORDS.contains(&s)
}

fn field_name(hdl: &str) -> String {
    let mut s: String = hdl.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if s.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(true) {
        s.insert(0, '_');
    }
    if KEYWORDS.contains(&s.as_str()) {
        s.push('_');
    }
    s
}

fn type_name(path: &[String]) -> String {
    let mut out = String::new();
    for (i, seg) in path.iter().enumerate() {
        let seg = field_name(seg);
        let mut chars = seg.chars();
        if let Some(c) = chars.next() {
            if i == 0 {
                out.push(c.to_ascii_uppercase());
            } else {
                out.push('_');
                out.push(c);
            }
            out.extend(chars);
        }
    }
    out
}

struct Gen {
    out: String,
    emitted: HashSet<String>,
}

impl Gen {
    fn module(&mut self, node: &Value, path: &[String]) -> String {
        let ty = type_name(path);
        if !self.emitted.insert(ty.clone()) {
            return ty;
        }
        // Inside a generate block element, constants are genvars and
        // localparams that not every simulator exposes; leave them out so the
        // bindings stay portable.
        let in_generate = path.last().map(|s| s.ends_with(']')).unwrap_or(false);
        let children = node.get("children").and_then(Value::as_array).cloned().unwrap_or_default();
        let mut fields = Vec::new();
        let mut binds = Vec::new();
        let mut used = HashSet::new();
        for c in &children {
            let name = c["name"].as_str().unwrap_or("").to_string();
            let kind = c["kind"].as_str().unwrap_or("Unknown");
            // Icarus exposes its internal scopes (`$ivl_for_loop0`,
            // `$unm_blk_3`); they exist on no other simulator.
            if name.starts_with('$') {
                continue;
            }
            let mut f = field_name(&name);
            while !used.insert(f.clone()) {
                f.push('_');
            }
            match kind {
                "Module" | "Struct" | "Package" => {
                    let mut sub = path.to_vec();
                    sub.push(name.clone());
                    let sub_ty = self.module(c, &sub);
                    fields.push(format!("    /// `{name}`\n    pub {f}: {sub_ty},"));
                    binds.push(format!("            {f}: {sub_ty}::from_module(module.module({name:?})?)?,"));
                }
                "GenArray" => {
                    fields.push(format!("    /// `{name}` (generate array; index at run time)\n    pub {f}: Module,"));
                    binds.push(format!("            {f}: module.module({name:?})?,"));
                }
                "Array" => {
                    let (lo, hi) = match c.get("range").and_then(Value::as_array) {
                        Some(r) if r.len() == 2 => {
                            let a = r[0].as_i64().unwrap_or(0);
                            let b = r[1].as_i64().unwrap_or(0);
                            (a.min(b), a.max(b))
                        }
                        _ => (0, c["width"].as_i64().unwrap_or(0) - 1),
                    };
                    fields.push(format!("    /// `{name}` [{lo}..={hi}]\n    pub {f}: Vec<Signal>,"));
                    binds.push(format!("            {f}: bind_array(&module, {name:?}, {lo}, {hi})?,"));
                }
                "Unknown" => {}
                _ if in_generate && c["is_const"].as_bool().unwrap_or(false) => {}
                _ => {
                    let width = c["width"].as_u64().unwrap_or(0);
                    let ty_note = c["type"].as_str().unwrap_or("");
                    fields.push(format!("    /// `{name}`: {kind} {ty_note}, {width} bit(s)\n    pub {f}: Signal,"));
                    binds.push(format!("            {f}: bind_signal(&module, {name:?}, {width})?,"));
                }
            }
        }
        let _ = writeln!(self.out, "/// `{}`", node["path"].as_str().unwrap_or(""));
        let _ = writeln!(
            self.out,
            "#[derive(Clone)]\npub struct {ty} {{\n    pub module: Module,\n{}\n}}\n",
            fields.join("\n")
        );
        let _ = writeln!(
            self.out,
            "impl {ty} {{\n    pub fn from_module(module: Module) -> Result<Self> {{\n        Ok(Self {{\n{}\n            module,\n        }})\n    }}\n}}\n",
            binds.join("\n")
        );
        ty
    }
}

/// Generate bindings from the hierarchy dump; `sources` are scanned for
/// `typedef enum` and `typedef struct packed` declarations.
pub fn generate(json: &str, top_type: Option<&str>, sources: &[std::path::PathBuf]) -> Result<String, String> {
    let root: Value = serde_json::from_str(json).map_err(|e| format!("bad hierarchy JSON: {e}"))?;
    let name = root["name"].as_str().unwrap_or("dut").to_string();
    let mut defs = crate::typedefs::Typedefs::default();
    for src in sources {
        if let Ok(text) = std::fs::read_to_string(src) {
            crate::typedefs::parse(&text, &mut defs);
        }
    }
    let mut g = Gen { out: String::new(), emitted: HashSet::new() };
    g.out.push_str(
        "// Generated by `rivet bindgen`. Do not edit; regenerate when the design changes.\n\
         #![allow(non_snake_case, non_camel_case_types, dead_code, unused_imports, clippy::all)]\n\n\
         use rivet::{Bind, IntoLogicVec, LogicVec, Module, Result, Signal};\n\n\
         fn bind_signal(m: &Module, name: &str, width: u32) -> Result<Signal> {\n    \
             let s = m.signal(name)?;\n    \
             if width != 0 && s.width() != width {\n        \
                 return Err(rivet::Error::Msg(format!(\n            \"{}: expected width {width}, design has {}; regenerate bindings\",\n            s.path(),\n            s.width()\n        )));\n    \
             }\n    \
             Ok(s)\n\
         }\n\n\
         fn bind_array(m: &Module, name: &str, lo: i64, hi: i64) -> Result<Vec<Signal>> {\n    \
             let a = m.signal(name)?;\n    \
             (lo..=hi).map(|i| a.index(i)).collect()\n\
         }\n\n",
    );
    let ty = g.module(&root, &[name]);
    let top = top_type.map(str::to_string).unwrap_or_else(|| "Dut".to_string());
    if top != ty {
        let _ = writeln!(g.out, "/// Alias for the top-level `{ty}`.\npub type {top} = {ty};\n");
    }
    let _ = writeln!(g.out, "impl Bind for {ty} {{\n    fn bind(root: Module) -> Result<Self> {{\n        Self::from_module(root)\n    }}\n}}\n");
    let _ = writeln!(
        g.out,
        "impl {ty} {{\n    /// The design below this instance as an indented tree.\n    pub fn hierarchy(&self) -> String {{\n        rivet::test::format_hierarchy(self.module)\n    }}\n}}\n"
    );
    if !defs.enums.is_empty() || !defs.structs.is_empty() || !defs.skipped.is_empty() {
        g.out.push_str("// ---- typedefs from the design sources ----\n\n");
        g.out.push_str(&crate::typedefs::generate(&defs));
    }
    Ok(g.out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typedefs_from_sources_parse_as_rust() {
        let dir = std::env::temp_dir().join(format!("rivet-bindgen-types-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("types.sv");
        std::fs::write(
            &src,
            "typedef enum logic [1:0] { IDLE, BUSY = 2, DONE } state_t;\n\
             typedef struct packed { state_t st; logic [7:0] data; logic ok; } rec_t;\n\
             typedef struct packed { rec_t r; logic [99:0] big; } wide_t;\n",
        )
        .unwrap();
        let code = generate(JSON, None, &[src]).unwrap();
        syn::parse_file(&code).expect("generated code with typedefs parses as Rust");
        assert!(code.contains("pub enum State {"), "{code}");
        assert!(code.contains("pub struct Rec {"), "{code}");
        assert!(code.contains("pub struct Wide {"), "{code}");
        assert!(code.contains("pub big: LogicVec,"), "{code}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    const JSON: &str = r#"{"name": "top", "path": "top", "kind": "Module", "width": 0, "is_const": false, "signed": false, "type": "module", "children": [
  {"name": "clk", "path": "top.clk", "kind": "Logic", "width": 1, "is_const": false, "signed": false, "type": "net"},
  {"name": "type", "path": "top.type", "kind": "LogicVec", "width": 8, "is_const": false, "signed": true, "type": "reg"},
  {"name": "1bad", "path": "top.1bad", "kind": "Real", "width": 64, "is_const": false, "signed": false, "type": "real"},
  {"name": "mem", "path": "top.mem", "kind": "Array", "width": 4, "is_const": false, "signed": false, "type": "reg array", "element": {"kind": "LogicVec", "width": 8}, "range": [3, 0]},
  {"name": "WIDTH", "path": "top.WIDTH", "kind": "LogicVec", "width": 32, "is_const": true, "signed": false, "type": "parameter"},
  {"name": "u_sub", "path": "top.u_sub", "kind": "Module", "width": 0, "is_const": false, "signed": false, "type": "module", "children": [
    {"name": "x", "path": "top.u_sub.x", "kind": "Integer", "width": 32, "is_const": false, "signed": true, "type": "integer"}
  ]},
  {"name": "gen", "path": "top.gen", "kind": "GenArray", "width": 0, "is_const": false, "signed": false, "type": "generate array"},
  {"name": "gen[0]", "path": "top.gen[0]", "kind": "Module", "width": 0, "is_const": false, "signed": false, "type": "module", "children": [
    {"name": "gi", "path": "top.gen[0].gi", "kind": "LogicVec", "width": 32, "is_const": true, "signed": false, "type": "parameter"},
    {"name": "tap", "path": "top.gen[0].tap", "kind": "LogicVec", "width": 8, "is_const": false, "signed": false, "type": "reg"}
  ]},
  {"name": "weird", "path": "top.weird", "kind": "Unknown", "width": 0, "is_const": false, "signed": false, "type": "?"},
  {"name": "$ivl_for_loop0", "path": "top.$ivl_for_loop0", "kind": "Module", "width": 0, "is_const": false, "signed": false, "type": "module", "children": []}
]}"#;

    #[test]
    fn generates_valid_rust() {
        let code = generate(JSON, None, &[]).unwrap();
        syn::parse_file(&code).expect("generated code parses as Rust");
        assert!(code.contains("pub fn hierarchy(&self) -> String"));
        assert!(code.contains("pub struct Top {"));
        assert!(code.contains("pub clk: Signal,"));
        assert!(code.contains("pub type_: Signal,"), "keyword field gets a trailing underscore");
        assert!(code.contains("pub _1bad: Signal,"), "leading digit gets a prefix");
        assert!(code.contains("pub mem: Vec<Signal>,"));
        assert!(code.contains("bind_array(&module, \"mem\", 0, 3)"));
        assert!(code.contains("pub WIDTH: Signal,"));
        assert!(code.contains("bind_signal(&module, \"WIDTH\", 32)"));
        assert!(code.contains("pub struct Top_u_sub {"));
        assert!(code.contains("pub u_sub: Top_u_sub,"));
        assert!(code.contains("pub gen: Module,"), "generate arrays are indexed at run time");
        assert!(code.contains("pub struct Top_gen_0_ {"));
        assert!(code.contains("pub tap: Signal,"));
        assert!(!code.contains("\"gi\""), "genvar inside a generate element is skipped");
        assert!(!code.contains("weird"), "unknown kinds are skipped");
        assert!(!code.contains("ivl_for_loop0"), "Icarus internal scopes are skipped");
        assert!(code.contains("pub type Dut = Top;"));
        assert!(code.contains("impl Bind for Top"));
    }

    #[test]
    fn custom_top_type_and_bad_json() {
        let code = generate(JSON, Some("Top"), &[]).unwrap();
        assert!(!code.contains("pub type Top = Top;"));
        assert!(generate("{not json", None, &[]).is_err());
        assert_eq!(field_name("a-b c"), "a_b_c");
        assert_eq!(field_name("self"), "self_");
        assert_eq!(type_name(&["dff".into(), "u_x".into()]), "Dff_u_x");
    }
}
