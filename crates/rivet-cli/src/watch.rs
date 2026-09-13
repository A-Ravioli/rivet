//! `rivet watch`: rerun the tests whenever a source file changes.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// A cheap fingerprint of a set of files: count plus newest mtime plus
/// total size, which changes on any save, add or delete.
pub fn fingerprint(roots: &[PathBuf]) -> (usize, SystemTime, u64) {
    let mut count = 0;
    let mut newest = SystemTime::UNIX_EPOCH;
    let mut size = 0u64;
    fn visit(p: &Path, count: &mut usize, newest: &mut SystemTime, size: &mut u64) {
        if p.is_dir() {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(name, "target" | "sim_build" | ".git" | "obj_dir") {
                return;
            }
            if let Ok(rd) = std::fs::read_dir(p) {
                for e in rd.flatten() {
                    visit(&e.path(), count, newest, size);
                }
            }
        } else if let Ok(md) = p.metadata() {
            *count += 1;
            *size += md.len();
            if let Ok(m) = md.modified() {
                if m > *newest {
                    *newest = m;
                }
            }
        }
    }
    for r in roots {
        visit(r, &mut count, &mut newest, &mut size);
    }
    (count, newest, size)
}

/// Block until the fingerprint of `roots` changes (polling every 300 ms).
pub fn wait_for_change(roots: &[PathBuf]) {
    let start = fingerprint(roots);
    loop {
        std::thread::sleep(Duration::from_millis(300));
        if fingerprint(roots) != start {
            // Let editors finish writing.
            std::thread::sleep(Duration::from_millis(200));
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_changes_on_edit_and_add() {
        let dir = std::env::temp_dir().join(format!("rivet-watch-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("target")).unwrap();
        std::fs::write(dir.join("a.sv"), "module a; endmodule").unwrap();
        std::fs::write(dir.join("target").join("ignored"), "x").unwrap();
        let roots = vec![dir.clone()];
        let f1 = fingerprint(&roots);
        assert_eq!(f1.0, 1, "target/ is skipped");
        std::fs::write(dir.join("a.sv"), "module a; wire w; endmodule").unwrap();
        let f2 = fingerprint(&roots);
        assert_ne!(f1, f2);
        std::fs::write(dir.join("b.sv"), "").unwrap();
        let f3 = fingerprint(&roots);
        assert_eq!(f3.0, 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
