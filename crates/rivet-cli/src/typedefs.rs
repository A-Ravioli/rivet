//! Typed views of SystemVerilog `typedef enum` and `typedef struct packed`
//! declarations, generated from the design sources for `rivet bindgen`.
//!
//! Simulators expose packed values as plain vectors; the source tells us
//! the names. Declarations whose widths depend on parameters are skipped
//! with a comment.

use std::fmt::Write as _;

#[derive(Debug, Clone, PartialEq)]
pub struct EnumDef {
    pub name: String,
    pub width: u32,
    pub variants: Vec<(String, u64)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructDef {
    pub name: String,
    /// Fields from MSB to LSB: `(name, width, type name if a known typedef)`.
    pub fields: Vec<(String, u32, Option<String>)>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Typedefs {
    pub enums: Vec<EnumDef>,
    pub structs: Vec<StructDef>,
    pub skipped: Vec<String>,
}

impl Typedefs {
    fn width_of(&self, ty: &str) -> Option<u32> {
        self.enums
            .iter()
            .find(|e| e.name == ty)
            .map(|e| e.width)
            .or_else(|| self.structs.iter().find(|s| s.name == ty).map(|s| s.fields.iter().map(|f| f.1).sum()))
    }
}

fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '/' && chars.peek() == Some(&'/') {
            for c in chars.by_ref() {
                if c == '\n' {
                    out.push('\n');
                    break;
                }
            }
        } else if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut prev = ' ';
            for c in chars.by_ref() {
                if prev == '*' && c == '/' {
                    break;
                }
                prev = c;
            }
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

/// Evaluate an integer expression of literals with `+ - * / ( )` and SV
/// sized literals (`4'd3`, `'h1F`, `8'b1010`, `2_000`).
pub fn eval_int(expr: &str) -> Option<i128> {
    let toks = tokenize(expr)?;
    let mut pos = 0;
    let v = parse_sum(&toks, &mut pos)?;
    if pos == toks.len() {
        Some(v)
    } else {
        None
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(i128),
    Op(char),
}

fn tokenize(s: &str) -> Option<Vec<Tok>> {
    let mut out = Vec::new();
    let cs: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c.is_whitespace() {
            i += 1;
        } else if "+-*/()".contains(c) {
            out.push(Tok::Op(c));
            i += 1;
        } else if c.is_ascii_digit() || c == '\'' {
            // [size]'[sb]base digits | decimal
            let start = i;
            while i < cs.len() && cs[i].is_ascii_digit() {
                i += 1;
            }
            if i < cs.len() && cs[i] == '\'' {
                i += 1;
                if i < cs.len() && (cs[i] == 's' || cs[i] == 'S') {
                    i += 1;
                }
                let base = cs.get(i)?.to_ascii_lowercase();
                i += 1;
                let dstart = i;
                while i < cs.len() && (cs[i].is_ascii_alphanumeric() || cs[i] == '_') {
                    i += 1;
                }
                let digits: String = cs[dstart..i].iter().filter(|c| **c != '_').collect();
                let radix = match base {
                    'b' => 2,
                    'o' => 8,
                    'd' => 10,
                    'h' => 16,
                    _ => return None,
                };
                out.push(Tok::Num(i128::from_str_radix(&digits, radix).ok()?));
            } else {
                let digits: String = cs[start..i].iter().filter(|c| **c != '_').collect();
                out.push(Tok::Num(digits.parse().ok()?));
                let _ = start;
            }
        } else if c == '_' {
            i += 1;
        } else {
            return None;
        }
    }
    Some(out)
}

fn parse_sum(t: &[Tok], pos: &mut usize) -> Option<i128> {
    let mut v = parse_prod(t, pos)?;
    while let Some(Tok::Op(op)) = t.get(*pos) {
        match op {
            '+' => {
                *pos += 1;
                v += parse_prod(t, pos)?;
            }
            '-' => {
                *pos += 1;
                v -= parse_prod(t, pos)?;
            }
            _ => break,
        }
    }
    Some(v)
}

fn parse_prod(t: &[Tok], pos: &mut usize) -> Option<i128> {
    let mut v = parse_atom(t, pos)?;
    while let Some(Tok::Op(op)) = t.get(*pos) {
        match op {
            '*' => {
                *pos += 1;
                v *= parse_atom(t, pos)?;
            }
            '/' => {
                *pos += 1;
                let d = parse_atom(t, pos)?;
                if d == 0 {
                    return None;
                }
                v /= d;
            }
            _ => break,
        }
    }
    Some(v)
}

fn parse_atom(t: &[Tok], pos: &mut usize) -> Option<i128> {
    match t.get(*pos)? {
        Tok::Num(n) => {
            *pos += 1;
            Some(*n)
        }
        Tok::Op('(') => {
            *pos += 1;
            let v = parse_sum(t, pos)?;
            if t.get(*pos) == Some(&Tok::Op(')')) {
                *pos += 1;
                Some(v)
            } else {
                None
            }
        }
        Tok::Op('-') => {
            *pos += 1;
            Some(-parse_atom(t, pos)?)
        }
        _ => None,
    }
}

/// Width of a packed dimension list like `[7:0]` or `[3:0][7:0]`; `None`
/// if any bound is not a literal expression.
fn dims_width(dims: &str) -> Option<u32> {
    let mut w = 1u32;
    let mut rest = dims.trim();
    while let Some(open) = rest.find('[') {
        let close = rest[open..].find(']')? + open;
        let inner = &rest[open + 1..close];
        let (a, b) = inner.split_once(':')?;
        let (a, b) = (eval_int(a)?, eval_int(b)?);
        w = w.checked_mul(((a - b).abs() + 1) as u32)?;
        rest = &rest[close + 1..];
    }
    Some(w)
}

/// Split a type text like `logic [7:0]` or `my_t` into base type and dims.
fn split_type(ty: &str) -> (String, String) {
    match ty.find('[') {
        Some(i) => (ty[..i].trim().to_string(), ty[i..].trim().to_string()),
        None => (ty.trim().to_string(), String::new()),
    }
}

fn base_width(base: &str) -> Option<u32> {
    let base = base.split_whitespace().filter(|w| !matches!(*w, "unsigned" | "signed")).collect::<Vec<_>>().join(" ");
    match base.as_str() {
        "logic" | "bit" | "reg" | "wire" => Some(1),
        "byte" => Some(8),
        "shortint" => Some(16),
        "int" | "integer" => Some(32),
        "longint" => Some(64),
        "" => Some(32),
        _ => None,
    }
}

/// Parse every `typedef enum` and `typedef struct packed` in `text`.
pub fn parse(text: &str, into: &mut Typedefs) {
    let text = strip_comments(text);
    let mut search = 0;
    while let Some(off) = text[search..].find("typedef") {
        let start = search + off;
        search = start + 7;
        let rest = text[start + 7..].trim_start();
        // Boundary check: `typedef` must be a whole word.
        if start > 0 && text.as_bytes()[start - 1].is_ascii_alphanumeric() {
            continue;
        }
        let Some(open) = rest.find('{') else { continue };
        let head = rest[..open].trim();
        let Some(close) = rest[open..].find('}') else { continue };
        let body = &rest[open + 1..open + close];
        let after = &rest[open + close + 1..];
        let Some(semi) = after.find(';') else { continue };
        let name = after[..semi].trim();
        let name = name.split_whitespace().next().unwrap_or("").trim_matches(|c| c == '[' || c == ']');
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        if let Some(base) = head.strip_prefix("enum") {
            let base = base.trim();
            let (b, dims) = split_type(base);
            let width = match (base_width(&b), dims_width(&dims)) {
                (Some(bw), Some(dw)) => bw * dw,
                _ => match into.width_of(&b) {
                    Some(w) => w,
                    None => {
                        into.skipped.push(format!("enum {name}: width of `{base}` is not a literal"));
                        continue;
                    }
                },
            };
            let mut variants = Vec::new();
            let mut next = 0u64;
            let mut ok = true;
            for item in body.split(',') {
                let item = item.trim();
                if item.is_empty() {
                    continue;
                }
                let (vname, value) = match item.split_once('=') {
                    Some((n, v)) => match eval_int(v.trim()) {
                        Some(x) if x >= 0 => (n.trim(), x as u64),
                        _ => {
                            ok = false;
                            break;
                        }
                    },
                    None => (item, next),
                };
                let vname = vname.trim();
                if vname.contains('[') || !vname.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    ok = false;
                    break;
                }
                variants.push((vname.to_string(), value));
                next = value + 1;
            }
            if !ok || variants.is_empty() {
                into.skipped.push(format!("enum {name}: unsupported variant list"));
                continue;
            }
            into.enums.push(EnumDef { name: name.to_string(), width, variants });
        } else if head.starts_with("struct") {
            if !head.contains("packed") {
                into.skipped.push(format!("struct {name}: unpacked structs have no bit layout"));
                continue;
            }
            let mut fields = Vec::new();
            let mut ok = true;
            for decl in body.split(';') {
                let decl = decl.trim();
                if decl.is_empty() {
                    continue;
                }
                // `<type> [dims] <name>` possibly with several names.
                let (ty_part, names) = match decl.rfind(|c: char| c.is_whitespace() || c == ']') {
                    Some(i) => (&decl[..=i], &decl[i + 1..]),
                    None => {
                        ok = false;
                        break;
                    }
                };
                let names_part = names.trim();
                let ty_part = ty_part.trim();
                // Names may be "a, b" when split across the type; handle the
                // common single-name case plus comma lists after the type.
                let (ty_text, name_list) = if ty_part.contains(',') {
                    let i = ty_part.find(',').unwrap();
                    let first_name_start = ty_part[..i].rfind(|c: char| c.is_whitespace() || c == ']').unwrap_or(0);
                    (ty_part[..=first_name_start].trim(), format!("{}{}", &ty_part[first_name_start + 1..], names_part))
                } else {
                    (ty_part, names_part.to_string())
                };
                let (base, dims) = split_type(ty_text);
                let (width, tyname) = match base_width(&base) {
                    Some(bw) => match dims_width(&dims) {
                        Some(dw) => (bw * dw, None),
                        None => {
                            ok = false;
                            break;
                        }
                    },
                    None => match into.width_of(&base) {
                        Some(w) if dims.is_empty() => (w, Some(base.clone())),
                        _ => {
                            ok = false;
                            break;
                        }
                    },
                };
                for n in name_list.split(',') {
                    let n = n.trim().trim_end_matches(',');
                    if n.is_empty() {
                        continue;
                    }
                    if n.contains('[') {
                        ok = false;
                        break;
                    }
                    fields.push((n.to_string(), width, tyname.clone()));
                }
            }
            if !ok || fields.is_empty() {
                into.skipped.push(format!("struct {name}: field widths are not all literals"));
                continue;
            }
            into.structs.push(StructDef { name: name.to_string(), fields });
        }
    }
}

fn rust_type_name(sv: &str) -> String {
    let base = sv.strip_suffix("_t").unwrap_or(sv);
    let mut out = String::new();
    let mut up = true;
    for c in base.chars() {
        if c == '_' {
            up = true;
        } else if up {
            out.push(c.to_ascii_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    if out.is_empty() {
        out.push_str("Anon");
    }
    if out.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
        out.insert(0, '_');
    }
    out
}

fn rust_field_name(sv: &str) -> String {
    let mut s = sv.to_string();
    if crate::bindgen::is_keyword(&s) {
        s.push('_');
    }
    s
}

/// Rust code for the typedefs.
pub fn generate(defs: &Typedefs) -> String {
    let mut out = String::new();
    for s in &defs.skipped {
        let _ = writeln!(out, "// bindgen skipped {s}");
    }
    if !defs.skipped.is_empty() {
        out.push('\n');
    }
    for e in &defs.enums {
        let ty = rust_type_name(&e.name);
        let _ = writeln!(out, "/// `typedef enum` `{}` ({} bits)", e.name, e.width);
        let _ = writeln!(out, "#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]\n#[repr(u64)]\npub enum {ty} {{");
        for (v, n) in &e.variants {
            let _ = writeln!(out, "    {} = {n},", rust_field_name(v));
        }
        let _ = writeln!(out, "}}\n");
        let _ = writeln!(out, "impl {ty} {{\n    pub const WIDTH: u32 = {};", e.width);
        let _ = writeln!(
            out,
            "    pub const ALL: &'static [{ty}] = &[{}];",
            e.variants.iter().map(|(v, _)| format!("{ty}::{}", rust_field_name(v))).collect::<Vec<_>>().join(", ")
        );
        let _ = writeln!(out, "    pub fn from_bits(v: u64) -> Option<{ty}> {{\n        match v {{");
        for (v, n) in &e.variants {
            let _ = writeln!(out, "            {n} => Some({ty}::{}),", rust_field_name(v));
        }
        let _ = writeln!(out, "            _ => None,\n        }}\n    }}");
        let _ = writeln!(out, "    pub fn bits(self) -> u64 {{\n        self as u64\n    }}");
        let _ = writeln!(out, "    pub fn name(self) -> &'static str {{\n        match self {{");
        for (v, _) in &e.variants {
            let _ = writeln!(out, "            {ty}::{} => \"{v}\",", rust_field_name(v));
        }
        let _ = writeln!(out, "        }}\n    }}");
        let _ = writeln!(
            out,
            "    /// Read a signal declared with this type.\n    pub fn from_signal(s: &Signal) -> Result<{ty}> {{\n        let v = s.get_u64()?;\n        {ty}::from_bits(v).ok_or_else(|| rivet::Error::Msg(format!(\"{{}} = {{v:#x}} is not a {} value\", s.path())))\n    }}\n}}\n",
            e.name
        );
        let _ = writeln!(out, "impl TryFrom<u64> for {ty} {{\n    type Error = rivet::Error;\n    fn try_from(v: u64) -> Result<{ty}> {{\n        {ty}::from_bits(v).ok_or_else(|| rivet::Error::Msg(format!(\"{{v:#x}} is not a {} value\")))\n    }}\n}}\n", e.name);
        let _ = writeln!(out, "impl IntoLogicVec for {ty} {{\n    fn into_logic_vec(self, width: u32) -> LogicVec {{\n        LogicVec::from_u64(width, self as u64)\n    }}\n}}\n");
        let _ = writeln!(out, "impl std::fmt::Display for {ty} {{\n    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{\n        f.write_str(self.name())\n    }}\n}}\n");
    }
    for s in &defs.structs {
        let ty = rust_type_name(&s.name);
        let total: u32 = s.fields.iter().map(|f| f.1).sum();
        let _ = writeln!(out, "/// `typedef struct packed` `{}` ({total} bits; first field is the MSB)", s.name);
        let _ = writeln!(out, "#[derive(Clone, Debug, PartialEq)]\npub struct {ty} {{");
        for (n, w, t) in &s.fields {
            let rt = field_rust_type(*w, t.as_deref(), defs);
            let _ = writeln!(out, "    /// {w} bit(s)\n    pub {}: {rt},", rust_field_name(n));
        }
        let _ = writeln!(out, "}}\n");
        let _ = writeln!(out, "impl {ty} {{\n    pub const WIDTH: u32 = {total};");
        // from_bits
        let _ = writeln!(out, "    pub fn from_bits(v: &LogicVec) -> Result<{ty}> {{\n        if v.width() != {total} {{\n            return Err(rivet::Error::Msg(format!(\"{}: expected {total} bits, got {{}}\", v.width())));\n        }}", s.name);
        let mut hi = total as i64 - 1;
        let mut inits = Vec::new();
        for (n, w, t) in &s.fields {
            let lo = hi - *w as i64 + 1;
            let f = rust_field_name(n);
            let expr = match t.as_deref().and_then(|t| defs.enums.iter().find(|e| e.name == t)) {
                Some(e) => format!(
                    "{}::from_bits(v.slice({hi}, {lo}).to_u64()?).ok_or_else(|| rivet::Error::Msg(\"{}.{n}: not a {} value\".into()))?",
                    rust_type_name(&e.name),
                    s.name,
                    e.name
                ),
                None => match t.as_deref().and_then(|t| defs.structs.iter().find(|x| x.name == t)) {
                    Some(sub) => format!("{}::from_bits(&v.slice({hi}, {lo}))?", rust_type_name(&sub.name)),
                    None if *w <= 64 => format!("v.slice({hi}, {lo}).to_u64()?"),
                    None => format!("v.slice({hi}, {lo})"),
                },
            };
            inits.push(format!("            {f}: {expr},"));
            hi = lo - 1;
        }
        let _ = writeln!(out, "        Ok({ty} {{\n{}\n        }})\n    }}", inits.join("\n"));
        // to_bits
        let _ = writeln!(out, "    pub fn to_bits(&self) -> LogicVec {{\n        let mut v = LogicVec::zeros({total});\n        let mut hi: u32 = {total};");
        for (n, w, t) in &s.fields {
            let f = rust_field_name(n);
            let part = match t.as_deref() {
                Some(t) if defs.enums.iter().any(|e| e.name == t) => {
                    format!("LogicVec::from_u64({w}, self.{f} as u64)")
                }
                Some(t) if defs.structs.iter().any(|x| x.name == t) => format!("self.{f}.to_bits()"),
                _ if *w <= 64 => format!("LogicVec::from_u64({w}, self.{f})"),
                _ => format!("self.{f}.clone()"),
            };
            let _ = writeln!(out, "        hi -= {w};\n        for i in 0..{w} {{\n            v.set_bit(hi + i, ({part}).bit(i));\n        }}");
        }
        let _ = writeln!(out, "        v\n    }}");
        let _ = writeln!(
            out,
            "    pub fn from_signal(s: &Signal) -> Result<{ty}> {{\n        {ty}::from_bits(&s.get())\n    }}"
        );
        let _ = writeln!(out, "    /// Deposit this value on a signal declared with this type.\n    pub fn set_on(&self, s: &Signal) {{\n        s.set(self.to_bits())\n    }}\n}}\n");
        let _ = writeln!(out, "impl IntoLogicVec for {ty} {{\n    fn into_logic_vec(self, width: u32) -> LogicVec {{\n        let mut v = self.to_bits();\n        v.resize(width);\n        v\n    }}\n}}\n");
    }
    out
}

fn field_rust_type(width: u32, ty: Option<&str>, defs: &Typedefs) -> String {
    if let Some(t) = ty {
        if defs.enums.iter().any(|e| e.name == t) || defs.structs.iter().any(|s| s.name == t) {
            return rust_type_name(t);
        }
    }
    if width <= 64 {
        "u64".into()
    } else {
        "LogicVec".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = r#"
// comment typedef enum { X } fake_t;
package p;
  typedef enum logic [1:0] { OP_ADD = 2'd0, OP_SUB, OP_MUL = 2'b11 } op_t;
  typedef enum { RED, GREEN, BLUE } color_t;
  typedef enum logic [W-1:0] { A, B } bad_t;
  typedef struct packed {
    logic [7:0] addr;   /* the address */
    op_t        op;
    logic       valid, last;
    logic [1:0][3:0] lanes;
  } cmd_t;
  typedef struct packed { cmd_t cmd; logic [71:0] payload; } wide_t;
  typedef struct { int a; } unpacked_t;
  typedef struct packed { logic [N:0] x; } param_t;
endpackage
"#;

    #[test]
    fn parses_enums_and_structs() {
        let mut d = Typedefs::default();
        parse(SRC, &mut d);
        assert_eq!(d.enums.len(), 2);
        assert_eq!(d.enums[0].name, "op_t");
        assert_eq!(d.enums[0].width, 2);
        assert_eq!(d.enums[0].variants, vec![("OP_ADD".into(), 0), ("OP_SUB".into(), 1), ("OP_MUL".into(), 3)]);
        assert_eq!(d.enums[1].width, 32);
        assert_eq!(d.structs.len(), 2);
        let cmd = &d.structs[0];
        assert_eq!(cmd.name, "cmd_t");
        assert_eq!(
            cmd.fields,
            vec![
                ("addr".into(), 8, None),
                ("op".into(), 2, Some("op_t".into())),
                ("valid".into(), 1, None),
                ("last".into(), 1, None),
                ("lanes".into(), 8, None)
            ]
        );
        assert_eq!(d.structs[1].fields[0], ("cmd".into(), 20, Some("cmd_t".into())));
        assert_eq!(d.skipped.len(), 3, "{:?}", d.skipped);
        assert!(d.skipped.iter().any(|s| s.contains("bad_t")));
        assert!(d.skipped.iter().any(|s| s.contains("unpacked_t")));
        assert!(d.skipped.iter().any(|s| s.contains("param_t")));
    }

    #[test]
    fn literals() {
        assert_eq!(eval_int("4'd3"), Some(3));
        assert_eq!(eval_int("'h1F"), Some(31));
        assert_eq!(eval_int("8'b1010_0000"), Some(160));
        assert_eq!(eval_int("(7+1)*2-1"), Some(15));
        assert_eq!(eval_int("W-1"), None);
        assert_eq!(dims_width("[3:0][7:0]"), Some(32));
        assert_eq!(dims_width("[0:3]"), Some(4));
        assert_eq!(rust_type_name("axi_cmd_t"), "AxiCmd");
        assert_eq!(rust_type_name("t"), "T");
    }

    #[test]
    fn generated_code_shape() {
        let mut d = Typedefs::default();
        parse(SRC, &mut d);
        let code = generate(&d);
        assert!(code.contains("pub enum Op {\n    OP_ADD = 0,\n    OP_SUB = 1,\n    OP_MUL = 3,\n}"), "{code}");
        assert!(code.contains("pub struct Cmd {"));
        assert!(code.contains("pub op: Op,"));
        assert!(code.contains("pub payload: LogicVec,"));
        assert!(code.contains("op: Op::from_bits(v.slice(11, 10).to_u64()?)"), "{code}");
        assert!(code.contains("addr: v.slice(19, 12).to_u64()?"), "{code}");
        assert!(code.contains("// bindgen skipped enum bad_t"));
    }
}
