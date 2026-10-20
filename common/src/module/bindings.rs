use std::collections::{HashMap, HashSet};

use super::{types::DeclarationInfo, ModuleSummary};
use crate::{parser::Declaration, TokenData};

#[derive(Clone, Default)]
pub struct ImportBinding {
    pub module: String,
    pub names: Vec<String>,
    pub wildcard: bool,
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
        .filter(|binding| {
            binding.public && (binding.wildcard || binding.names.iter().any(|n| n == name))
        })
        .find_map(|binding| exported(index, &binding.module, name, seen))
}

pub(super) fn exported_declarations(
    index: &HashMap<String, ModuleSummary>,
    module: &str,
) -> Vec<DeclarationInfo> {
    let mut names = HashSet::new();
    let mut seen = HashSet::new();
    let mut pending = vec![module];
    while let Some(path) = pending.pop() {
        if !seen.insert(path) {
            continue;
        }
        if let Some(summary) = index.get(path) {
            names.extend(summary.declarations.keys().cloned());
            for binding in summary.bindings.iter().filter(|b| b.public) {
                names.extend(binding.names.iter().cloned());
                if binding.wildcard {
                    pending.push(&binding.module);
                }
            }
        }
    }
    let mut declarations: Vec<_> = names
        .into_iter()
        .filter_map(|name| exported(index, module, &name, &mut HashSet::new()))
        .collect();
    declarations.sort_by(|a, b| a.name.cmp(&b.name));
    declarations
}

pub(super) fn selected_names(
    index: &HashMap<String, ModuleSummary>,
    binding: &ImportBinding,
) -> Vec<String> {
    if binding.wildcard {
        exported_declarations(index, &binding.module)
            .into_iter()
            .map(|d| d.name)
            .collect()
    } else {
        binding.names.clone()
    }
}

pub(crate) fn import_selections(
    index: &HashMap<String, ModuleSummary>,
    module: &str,
) -> HashMap<String, Vec<String>> {
    let mut selections: HashMap<String, Vec<String>> = HashMap::new();
    if let Some(summary) = index.get(module) {
        for binding in &summary.bindings {
            let names = selections.entry(binding.module.clone()).or_default();
            for name in selected_names(index, binding) {
                if exported(index, &binding.module, &name, &mut HashSet::new()).is_some()
                    && !names.contains(&name)
                {
                    names.push(name);
                }
            }
        }
    }
    selections
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
        for name in selected_names(index, binding) {
            if let Some(declaration) = exported(index, &binding.module, &name, &mut HashSet::new())
            {
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
    let mut pending = exported_declarations(index, entry);
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
                    .filter(|n| n.text(source) != "*")
                    .map(|n: &TokenData| n.text(source).into())
                    .collect(),
                wildcard: names.iter().any(|n| n.text(source) == "*"),
                public: public.is_some(),
            }),
            _ => None,
        })
        .collect()
}
