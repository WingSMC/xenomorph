use std::collections::{HashMap, HashSet};

use super::{types::DeclarationInfo, ModuleSummary};
use crate::{parser::Declaration, TokenData};

#[derive(Clone, Default)]
pub struct ImportBinding {
    pub module: String,
    pub names: Vec<String>,
    pub public: bool,
}

pub(super) fn exported(
    index: &HashMap<String, ModuleSummary>,
    module: &str,
    name: &str,
    seen: &mut HashSet<(String, String)>,
) -> Option<DeclarationInfo> {
    if !seen.insert((module.into(), name.into())) {
        return None;
    }
    let summary = index.get(module)?;
    if let Some(declaration) = summary.declarations.get(name) {
        return declaration.public.then(|| declaration.clone());
    }
    summary
        .bindings
        .iter()
        .filter(|binding| binding.public && binding.names.iter().any(|n| n == name))
        .find_map(|binding| exported(index, &binding.module, name, seen))
}

pub(crate) fn in_scope(
    index: &HashMap<String, ModuleSummary>,
    module: &str,
) -> Vec<DeclarationInfo> {
    let Some(summary) = index.get(module) else {
        return vec![];
    };
    let mut result: Vec<_> = summary.declarations.values().cloned().collect();
    for binding in &summary.bindings {
        for name in &binding.names {
            if let Some(declaration) = exported(index, &binding.module, name, &mut HashSet::new()) {
                result.push(declaration);
            }
        }
    }
    result
}

pub(super) fn reachable(
    index: &HashMap<String, ModuleSummary>,
    entry: &str,
) -> HashSet<(String, String)> {
    let mut pending = vec![];
    if let Some(summary) = index.get(entry) {
        pending.extend(summary.declarations.values().filter(|d| d.public).cloned());
        for binding in summary.bindings.iter().filter(|b| b.public) {
            for name in &binding.names {
                if let Some(d) = exported(index, &binding.module, name, &mut HashSet::new()) {
                    pending.push(d);
                }
            }
        }
    }
    let mut seen = HashSet::new();
    while let Some(declaration) = pending.pop() {
        if !seen.insert((declaration.module_path.clone(), declaration.name.clone())) {
            continue;
        }
        if let Some(summary) = index.get(&declaration.module_path) {
            let scope = in_scope(index, &declaration.module_path);
            for name in summary
                .dependencies
                .get(&declaration.name)
                .into_iter()
                .flatten()
            {
                if let Some(d) = scope.iter().find(|d| &d.name == name) {
                    pending.push(d.clone());
                }
            }
        }
    }
    seen
}

pub(crate) fn import_bindings(ast: &[Declaration], source: &str) -> Vec<ImportBinding> {
    ast.iter()
        .filter_map(|d| match d {
            Declaration::Import {
                path,
                names,
                public,
                ..
            } => Some(ImportBinding {
                module: path.join("/"),
                names: names
                    .iter()
                    .map(|n: &TokenData| n.text(source).into())
                    .collect(),
                public: public.is_some(),
            }),
            _ => None,
        })
        .collect()
}
