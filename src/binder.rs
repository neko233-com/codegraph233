use crate::model::{is_type_kind, RefRow, SymbolRow};
use std::collections::{HashMap, HashSet};

pub struct Binder {
    rows: Vec<SymbolRow>,
    by_qual: HashMap<String, Vec<usize>>,
    by_name: HashMap<String, Vec<usize>>,
    powershell_by_name: HashMap<String, Vec<usize>>,
    by_container_name: HashMap<String, Vec<usize>>,
    by_ns_name: HashMap<String, Vec<usize>>,
    type_by_simple: HashMap<String, Vec<usize>>,
    package_paths: HashSet<String>,
    by_id: HashMap<i64, usize>,
}

pub struct Resolution {
    pub row: usize,
    pub confidence: &'static str,
}

impl Binder {
    pub fn new(rows: Vec<SymbolRow>) -> Binder {
        let mut b = Binder {
            by_qual: HashMap::new(),
            by_name: HashMap::new(),
            powershell_by_name: HashMap::new(),
            by_container_name: HashMap::new(),
            by_ns_name: HashMap::new(),
            type_by_simple: HashMap::new(),
            package_paths: HashSet::new(),
            by_id: HashMap::new(),
            rows,
        };
        for (i, s) in b.rows.iter().enumerate() {
            b.by_qual.entry(s.qualname.clone()).or_default().push(i);
            b.by_name.entry(s.name.clone()).or_default().push(i);
            if s.lang == "powershell" {
                b.powershell_by_name.entry(s.name.to_ascii_lowercase()).or_default().push(i);
            }
            if !s.container.is_empty() {
                b.by_container_name
                    .entry(format!("{}|{}", s.container, s.name))
                    .or_default()
                    .push(i);
            }
            if !s.namespace.is_empty() {
                b.by_ns_name
                    .entry(format!("{}|{}", s.namespace, s.name))
                    .or_default()
                    .push(i);
                b.package_paths.insert(s.namespace.clone());
            }
            if is_type_kind(&s.kind) {
                b.type_by_simple.entry(s.name.clone()).or_default().push(i);
            }
            b.by_id.insert(s.id, i);
        }
        b
    }

    pub fn row(&self, i: usize) -> &SymbolRow {
        &self.rows[i]
    }

    pub fn row_by_id(&self, id: i64) -> Option<&SymbolRow> {
        self.by_id.get(&id).map(|&i| &self.rows[i])
    }

    pub fn resolve(&self, r: &RefRow, src: &SymbolRow) -> Option<Resolution> {
        if r.name.is_empty() {
            return None;
        }
        if r.kind == "dynamic-call" { return None; }
        if src.lang == "powershell" {
            let mut candidates: Vec<usize> = self.powershell_by_name.get(&r.name.to_ascii_lowercase())?.iter().copied()
                .filter(|i| r.kind != "calls" || matches!(self.rows[*i].kind.as_str(),"function"|"method"|"constructor"))
                .filter(|i| self.rows[*i].root == src.root).collect();
            if !r.receiver_type.is_empty() {
                candidates.retain(|i| self.rows[*i].container.eq_ignore_ascii_case(&r.receiver_type)
                    || self.rows[*i].container.rsplit('.').next().unwrap_or("").eq_ignore_ascii_case(&r.receiver_type));
            }
            let local: Vec<usize> = candidates.iter().copied().filter(|i| self.rows[*i].namespace == src.namespace && self.rows[*i].container == src.container).collect();
            if !local.is_empty() { candidates = local; }
            return if candidates.len() == 1 { Some(Resolution { row:candidates[0], confidence:"exact" }) } else { None };
        }
        // JVM facts contain exact owner/name/descriptors. Never substitute a
        // name/arity guess for a missing or ambiguous bytecode declaration.
        if src.lang == "jvm" {
            if r.kind == "dynamic-call" { return None; }
            let candidates = self.by_qual.get(&r.name)?;
            let mut matching: Vec<usize> = candidates.iter().copied()
                .filter(|i| self.rows[*i].lang == "jvm").collect();
            let local: Vec<usize> = matching.iter().copied()
                .filter(|i| self.rows[*i].root == src.root).collect();
            if !local.is_empty() { matching = local; }
            return if matching.len() == 1 {
                Some(Resolution { row: matching[0], confidence: "exact" })
            } else { None };
        }
        let want_callable = r.kind == "calls";
        let want_type = matches!(r.kind.as_str(), "constructs" | "inherits" | "embeds");

        if r.kind == "type_uses" {
            if let Some(i) = self.pick(self.type_by_simple.get(&r.name), r, src, false, true) {
                return Some(Resolution { row: i, confidence: "type" });
            }
            if let Some(i) = self.pick(self.by_name.get(&r.name), r, src, false, true) {
                return Some(Resolution { row: i, confidence: "name" });
            }
            return None;
        }

        if !r.receiver_type.is_empty() {
            if let Some(res) = self.resolve_receiver(r, src, want_callable, want_type) {
                return Some(res);
            }
            if matches!(src.lang.as_str(), "java"|"c"|"cpp"|"python"|"kotlin"|"swift"|"objc"|"dart"|"haxe") {
                return None;
            }
        }
        if !src.container.is_empty() {
            if let Some(i) = self.pick(
                self.by_container_name.get(&format!("{}|{}", src.container, r.name)),
                r,
                src,
                want_callable,
                want_type,
            ) {
                return Some(Resolution { row: i, confidence: "exact" });
            }
        }
        if !r.namespace_hint.is_empty() && r.namespace_hint != src.namespace {
            if let Some(i) = self.pick(
                self.by_ns_name.get(&format!("{}|{}", r.namespace_hint, r.name)),
                r,
                src,
                want_callable,
                want_type,
            ) {
                return Some(Resolution { row: i, confidence: "exact" });
            }
        }
        if !src.namespace.is_empty() {
            if let Some(i) = self.pick(
                self.by_ns_name.get(&format!("{}|{}", src.namespace, r.name)),
                r,
                src,
                want_callable,
                want_type,
            ) {
                return Some(Resolution { row: i, confidence: "exact" });
            }
        }
        if want_type {
            if let Some(i) = self.pick(self.type_by_simple.get(&r.name), r, src, false, true) {
                return Some(Resolution { row: i, confidence: "name" });
            }
        }
        let mut cands = self.by_name.get(&r.name).cloned().unwrap_or_default();
        if want_callable {
            cands.retain(|&i| {
                matches!(
                    self.rows[i].kind.as_str(),
                    "method" | "function" | "constructor" | "operator"
                )
            });
        }
        if want_type {
            cands.retain(|&i| is_type_kind(&self.rows[i].kind));
        }
        let multi = cands.len() > 1;
        self.pick(Some(&cands), r, src, want_callable, want_type)
            .map(|i| Resolution {
                row: i,
                confidence: if multi { "name-multi" } else { "name" },
            })
    }

    fn resolve_receiver(
        &self,
        r: &RefRow,
        src: &SymbolRow,
        want_callable: bool,
        want_type: bool,
    ) -> Option<Resolution> {
        let rt = r.receiver_type.trim_start_matches('*');
        if rt.is_empty() {
            return None;
        }
        if matches!(src.lang.as_str(), "java"|"c"|"cpp"|"python"|"kotlin"|"swift"|"objc"|"dart"|"haxe") {
            let mut owners = vec![rt.to_string()];
            if !rt.contains('.') && !src.namespace.is_empty() {
                owners.insert(0, format!("{}.{rt}",src.namespace));
            }
            for owner in &owners {
                for candidates in [self.by_container_name.get(&format!("{owner}|{}",r.name)), self.by_ns_name.get(&format!("{owner}|{}",r.name))] {
                    if let Some(i) = self.pick(candidates,r,src,want_callable,want_type) {
                        return Some(Resolution { row:i, confidence:"type" });
                    }
                }
            }
            let types = self.type_by_simple.get(rt)?;
            let types: Vec<_> = types.iter().filter(|i| self.rows[**i].lang == src.lang && self.rows[**i].root == src.root).collect();
            if types.len() != 1 { return None; }
            let owner = &self.rows[*types[0]].qualname;
            return self.pick(self.by_container_name.get(&format!("{owner}|{}",r.name)),r,src,want_callable,want_type)
                .map(|i| Resolution { row:i, confidence:"type" });
        }
        if rt.contains('/') || self.package_paths.contains(rt) {
            if let Some(i) = self.pick(
                self.by_ns_name.get(&format!("{rt}|{}", r.name)),
                r,
                src,
                want_callable,
                want_type,
            ) {
                return Some(Resolution { row: i, confidence: "exact" });
            }
            return None;
        }
        let simple = rt.rsplit('.').next().unwrap_or(rt);
        if simple.is_empty() {
            return None;
        }
        let mut type_cands: Vec<usize> = self
            .type_by_simple
            .get(simple)
            .cloned()
            .unwrap_or_default();
        if type_cands.is_empty() {
            if let Some(v) = self.by_qual.get(rt) {
                type_cands = v.clone();
            }
        }
        type_cands.sort_by_key(|&i| std::cmp::Reverse(self.score(i, r, src)));
        for ti in type_cands {
            let tq = &self.rows[ti].qualname;
            if let Some(i) = self.pick(
                self.by_container_name.get(&format!("{tq}|{}", r.name)),
                r,
                src,
                want_callable,
                want_type,
            ) {
                return Some(Resolution { row: i, confidence: "type" });
            }
        }
        if let Some(i) = self.pick(
            self.by_ns_name.get(&format!("{rt}|{}", r.name)),
            r,
            src,
            want_callable,
            want_type,
        ) {
            return Some(Resolution { row: i, confidence: "type" });
        }
        None
    }

    fn pick(
        &self,
        cands: Option<&Vec<usize>>,
        r: &RefRow,
        src: &SymbolRow,
        want_callable: bool,
        want_type: bool,
    ) -> Option<usize> {
        let cands = cands?;
        if cands.is_empty() {
            return None;
        }
        let mut filtered: Vec<usize> = cands
            .iter()
            .copied()
            .filter(|&i| {
                let c = &self.rows[i];
                if !src.root.is_empty() && !c.root.is_empty() && c.root != src.root {
                    return false;
                }
                if !src.lang.is_empty() && !c.lang.is_empty() && c.lang != src.lang {
                    return false;
                }
                if want_callable {
                    if !matches!(
                        c.kind.as_str(),
                        "method" | "function" | "constructor" | "operator"
                    ) {
                        return false;
                    }
                    if r.arity >= 0 && c.param_count >= 0 && r.arity != c.param_count {
                        return false;
                    }
                }
                if want_type && !is_type_kind(&c.kind) {
                    return false;
                }
                true
            })
            .collect();
        if filtered.is_empty() {
            if matches!(src.lang.as_str(),"java"|"c"|"cpp"|"python"|"kotlin"|"swift"|"objc"|"dart"|"haxe") { return None; }
            filtered = cands.clone();
        }
        let mut best = filtered[0];
        let mut best_score = self.score(best, r, src);
        let mut ambiguous = false;
        for &i in &filtered[1..] {
            let s = self.score(i, r, src);
            if s > best_score {
                best = i;
                best_score = s;
                ambiguous = false;
            } else if s == best_score {
                ambiguous = true;
            }
        }
        if ambiguous && matches!(src.lang.as_str(),"java"|"c"|"cpp"|"python"|"kotlin"|"swift"|"objc"|"dart"|"haxe") { return None; }
        Some(best)
    }

    fn score(&self, i: usize, r: &RefRow, src: &SymbolRow) -> i32 {
        let c = &self.rows[i];
        let mut s = 0;
        if !src.root.is_empty() && c.root == src.root {
            s += 8;
        }
        if !src.lang.is_empty() && c.lang == src.lang {
            s += 4;
        }
        if !src.namespace.is_empty() && c.namespace == src.namespace {
            s += 4;
        }
        if !src.container.is_empty() && c.container == src.container {
            s += 6;
        }
        if r.arity >= 0 && c.param_count >= 0 {
            if r.arity == c.param_count {
                s += 6;
            } else {
                s -= 3;
            }
        }
        if c.qualname.ends_with(&format!(".{}", r.name)) {
            s += 1;
        }
        s
    }
}
