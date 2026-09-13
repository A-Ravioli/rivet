//! `rivet cov report`: merge `coverage.json` files from shards and
//! parameter sets, print the table, and apply a threshold.

use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Default, Debug, Clone)]
pub struct Point {
    pub bins: Vec<(String, u64)>,
    pub out_of_range: u64,
    pub ignored: u64,
}

#[derive(Default, Debug, Clone)]
pub struct CrossCov {
    pub points: Vec<String>,
    pub total: u64,
    pub bins: Vec<(String, u64)>,
}

#[derive(Default, Debug, Clone)]
pub struct Group {
    pub points: BTreeMap<String, Point>,
    pub crosses: BTreeMap<String, CrossCov>,
    /// Definition order of points and crosses (for stable output).
    pub order: Vec<(bool, String)>,
}

#[derive(Default, Debug, Clone)]
pub struct Report {
    pub groups: BTreeMap<String, Group>,
    pub order: Vec<String>,
}

fn add_bin(bins: &mut Vec<(String, u64)>, name: &str, hits: u64) {
    match bins.iter_mut().find(|(n, _)| n == name) {
        Some((_, h)) => *h += hits,
        None => bins.push((name.to_string(), hits)),
    }
}

impl Report {
    /// Merge one `coverage.json` (hits add up by group, point and bin name).
    pub fn merge_json(&mut self, json: &str) -> Result<(), String> {
        let v: Value = serde_json::from_str(json).map_err(|e| format!("bad coverage JSON: {e}"))?;
        for g in v["groups"].as_array().cloned().unwrap_or_default() {
            let gname = g["name"].as_str().unwrap_or("?").to_string();
            if !self.order.contains(&gname) {
                self.order.push(gname.clone());
            }
            let group = self.groups.entry(gname).or_default();
            for p in g["points"].as_array().cloned().unwrap_or_default() {
                let pname = p["name"].as_str().unwrap_or("?").to_string();
                if !group.order.iter().any(|(c, n)| !*c && *n == pname) {
                    group.order.push((false, pname.clone()));
                }
                let point = group.points.entry(pname).or_default();
                point.out_of_range += p["out_of_range"].as_u64().unwrap_or(0);
                point.ignored += p["ignored"].as_u64().unwrap_or(0);
                for b in p["bins"].as_array().cloned().unwrap_or_default() {
                    add_bin(&mut point.bins, b["name"].as_str().unwrap_or("?"), b["hits"].as_u64().unwrap_or(0));
                }
            }
            for c in g["crosses"].as_array().cloned().unwrap_or_default() {
                let cname = c["name"].as_str().unwrap_or("?").to_string();
                if !group.order.iter().any(|(x, n)| *x && *n == cname) {
                    group.order.push((true, cname.clone()));
                }
                let cross = group.crosses.entry(cname).or_default();
                cross.points = c["points"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                    .unwrap_or_default();
                cross.total = cross.total.max(c["total"].as_u64().unwrap_or(0));
                for b in c["bins"].as_array().cloned().unwrap_or_default() {
                    add_bin(&mut cross.bins, b["name"].as_str().unwrap_or("?"), b["hits"].as_u64().unwrap_or(0));
                }
            }
        }
        Ok(())
    }

    pub fn merge_file(&mut self, path: &Path) -> Result<(), String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        self.merge_json(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// `(covered, total)` bins across every group.
    pub fn counts(&self) -> (u64, u64) {
        let mut covered = 0;
        let mut total = 0;
        for g in self.groups.values() {
            for p in g.points.values() {
                total += p.bins.len() as u64;
                covered += p.bins.iter().filter(|(_, h)| *h > 0).count() as u64;
            }
            for c in g.crosses.values() {
                total += c.total;
                covered += c.bins.iter().filter(|(_, h)| *h > 0).count() as u64;
            }
        }
        (covered, total)
    }

    pub fn percent(&self) -> f64 {
        let (c, t) = self.counts();
        if t == 0 {
            100.0
        } else {
            100.0 * c as f64 / t as f64
        }
    }

    pub fn render(&self) -> String {
        use std::fmt::Write as _;
        let mut s = String::new();
        for gname in &self.order {
            let g = &self.groups[gname];
            let (mut gc, mut gt) = (0u64, 0u64);
            for p in g.points.values() {
                gt += p.bins.len() as u64;
                gc += p.bins.iter().filter(|(_, h)| *h > 0).count() as u64;
            }
            for c in g.crosses.values() {
                gt += c.total;
                gc += c.bins.iter().filter(|(_, h)| *h > 0).count() as u64;
            }
            let pct = if gt == 0 { 100.0 } else { 100.0 * gc as f64 / gt as f64 };
            let _ = writeln!(s, "covergroup {gname} : {gc}/{gt} bins ({pct:.1}%)");
            for (is_cross, name) in &g.order {
                if *is_cross {
                    let c = &g.crosses[name];
                    let hit = c.bins.iter().filter(|(_, h)| *h > 0).count();
                    let _ = writeln!(s, "  cross {:<24} {hit}/{}", name, c.total);
                } else {
                    let p = &g.points[name];
                    let hit = p.bins.iter().filter(|(_, h)| *h > 0).count();
                    let _ = writeln!(s, "  point {:<24} {hit}/{}", name, p.bins.len());
                    for (b, h) in &p.bins {
                        let _ = writeln!(s, "    {:<26} {h:>8}{}", b, if *h == 0 { "  <-- uncovered" } else { "" });
                    }
                    if p.out_of_range > 0 {
                        let _ = writeln!(s, "    (out of range)             {:>8}", p.out_of_range);
                    }
                }
            }
        }
        let (c, t) = self.counts();
        let _ = writeln!(s, "TOTAL {c}/{t} bins ({:.2}%)", self.percent());
        s
    }

    /// The merged report in the same JSON shape the harness writes.
    pub fn to_json(&self) -> String {
        let groups: Vec<Value> = self
            .order
            .iter()
            .map(|gname| {
                let g = &self.groups[gname];
                let points: Vec<Value> = g
                    .order
                    .iter()
                    .filter(|(c, _)| !*c)
                    .map(|(_, n)| {
                        let p = &g.points[n];
                        serde_json::json!({
                            "name": n,
                            "out_of_range": p.out_of_range,
                            "ignored": p.ignored,
                            "bins": p.bins.iter().map(|(b, h)| serde_json::json!({"name": b, "hits": h})).collect::<Vec<_>>(),
                        })
                    })
                    .collect();
                let crosses: Vec<Value> = g
                    .order
                    .iter()
                    .filter(|(c, _)| *c)
                    .map(|(_, n)| {
                        let c = &g.crosses[n];
                        serde_json::json!({
                            "name": n,
                            "points": c.points,
                            "total": c.total,
                            "bins": c.bins.iter().map(|(b, h)| serde_json::json!({"name": b, "hits": h})).collect::<Vec<_>>(),
                        })
                    })
                    .collect();
                serde_json::json!({"name": gname, "points": points, "crosses": crosses})
            })
            .collect();
        serde_json::json!({ "groups": groups }).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = r#"{"groups":[{"name":"g","points":[{"name":"len","out_of_range":1,"ignored":0,"bins":[{"name":"one","hits":1},{"name":"few","hits":0}]}],"crosses":[{"name":"x","points":["len","size"],"total":4,"bins":[{"name":"one x s0","hits":2}]}]}]}"#;
    const B: &str = r#"{"groups":[{"name":"g","points":[{"name":"len","out_of_range":0,"ignored":2,"bins":[{"name":"one","hits":3},{"name":"few","hits":5}]}],"crosses":[{"name":"x","points":["len","size"],"total":4,"bins":[{"name":"few x s1","hits":1}]}]},{"name":"h","points":[{"name":"p","bins":[{"name":"b","hits":0}]}],"crosses":[]}]}"#;

    #[test]
    fn merge_and_render() {
        let mut r = Report::default();
        r.merge_json(A).unwrap();
        assert_eq!(r.counts(), (2, 6));
        r.merge_json(B).unwrap();
        let g = &r.groups["g"];
        assert_eq!(g.points["len"].bins, vec![("one".to_string(), 4), ("few".to_string(), 5)]);
        assert_eq!(g.points["len"].out_of_range, 1);
        assert_eq!(g.points["len"].ignored, 2);
        assert_eq!(g.crosses["x"].bins.len(), 2);
        assert_eq!(r.counts(), (4, 7));
        let text = r.render();
        assert!(text.contains("covergroup g : 4/6 bins (66.7%)"), "{text}");
        assert!(text.contains("    b ") && text.contains("<-- uncovered"), "{text}");
        assert!(text.contains("  point p                        0/1"), "{text}");
        assert!(text.contains("TOTAL 4/7 bins (57.14%)"), "{text}");
        let again: Report = {
            let mut x = Report::default();
            x.merge_json(&r.to_json()).unwrap();
            x
        };
        assert_eq!(again.counts(), r.counts());
        assert!(r.merge_json("nope").is_err());
        assert_eq!(Report::default().percent(), 100.0);
    }
}
