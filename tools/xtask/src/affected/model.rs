//! The classifier's model of the workspace: which package owns each
//! path, how workspace packages depend on one another, and what each
//! CI domain declares.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::error::XtaskError;

/// One CI domain's declaration from the workspace manifest.
pub(crate) struct Domain {
    /// Paths the domain reaches directly.
    pub(crate) paths: Vec<String>,
    /// Non-code inputs the domain's builds and tests consume.
    pub(crate) extra_paths: Vec<String>,
    /// Every workspace package whose change can reach the domain: the
    /// declared roots plus everything they transitively depend on.
    pub(crate) package_closure: BTreeSet<String>,
}

/// Everything classification needs, extracted once from cargo metadata.
pub(crate) struct ClassifierModel {
    /// Workspace member name and manifest directory, longest dirs first
    /// so ownership resolves to the most specific package.
    members: Vec<(String, PathBuf)>,
    /// Paths that reach no build at all.
    inert: Vec<String>,
    pub(crate) domains: BTreeMap<String, Domain>,
}

impl ClassifierModel {
    /// Builds the model from `cargo metadata` JSON.
    ///
    /// # Errors
    ///
    /// Returns a typed [`XtaskError`] when the metadata does not carry
    /// the workspace shape this classifier needs — the caller treats
    /// that as "run everything".
    pub(crate) fn from_workspace(metadata: &serde_json::Value) -> Result<Self, XtaskError> {
        let workspace_root = string_at(metadata, "workspace_root")?;
        let packages = metadata
            .get("packages")
            .and_then(|value| value.as_array())
            .ok_or_else(|| shape_error("packages"))?;
        let member_ids: BTreeSet<&str> = metadata
            .get("workspace_members")
            .and_then(|value| value.as_array())
            .ok_or_else(|| shape_error("workspace_members"))?
            .iter()
            .filter_map(|value| value.as_str())
            .collect();

        let mut members = Vec::new();
        let mut dependencies: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut names = BTreeSet::new();
        for package in packages {
            let id = string_at(package, "id")?;
            if !member_ids.contains(id.as_str()) {
                continue;
            }
            let name = string_at(package, "name")?;
            names.insert(name.clone());
            let manifest = PathBuf::from(string_at(package, "manifest_path")?);
            let dir = manifest
                .parent()
                .map(|parent| relative_to(parent, &workspace_root))
                .ok_or_else(|| shape_error("manifest_path"))?;
            members.push((name.clone(), dir));
            let declared = package
                .get("dependencies")
                .and_then(|value| value.as_array())
                .ok_or_else(|| shape_error("dependencies"))?
                .iter()
                .filter_map(|dependency| dependency.get("name"))
                .filter_map(|value| value.as_str())
                .map(str::to_owned)
                .collect();
            dependencies.insert(name, declared);
        }
        for declared in dependencies.values_mut() {
            declared.retain(|name| names.contains(name));
        }
        members.sort_by_key(|(_, dir)| std::cmp::Reverse(dir.components().count()));

        let ci = metadata
            .pointer("/metadata/ci")
            .ok_or_else(|| shape_error("workspace.metadata.ci"))?;
        let inert = string_list(ci.get("inert-paths"));
        let mut domains = BTreeMap::new();
        if let Some(declared) = ci.get("domains").and_then(|value| value.as_object()) {
            for (name, table) in declared {
                let roots = string_list(table.get("depends-on-packages"));
                domains.insert(
                    name.clone(),
                    Domain {
                        paths: string_list(table.get("paths")),
                        extra_paths: string_list(table.get("extra-paths")),
                        package_closure: dependency_closure(&roots, &dependencies),
                    },
                );
            }
        }
        Ok(Self {
            members,
            inert,
            domains,
        })
    }

    /// The workspace package whose directory owns this file, most
    /// specific directory first.
    pub(crate) fn owning_package(&self, file: &str) -> Option<&str> {
        self.members
            .iter()
            .find(|(_, dir)| super::dir_owns(dir, file))
            .map(|(name, _)| name.as_str())
    }

    /// True when the file reaches no build at all.
    pub(crate) fn is_inert(&self, file: &str) -> bool {
        self.inert.iter().any(|rule| {
            if let Some(suffix) = rule.strip_prefix('*') {
                file.ends_with(suffix)
            } else {
                file.starts_with(rule.as_str())
            }
        })
    }

    #[cfg(test)]
    pub(crate) fn synthetic(
        members: Vec<(String, PathBuf)>,
        inert: Vec<String>,
        domains: BTreeMap<String, Domain>,
    ) -> Self {
        let mut members = members;
        members.sort_by_key(|(_, dir)| std::cmp::Reverse(dir.components().count()));
        Self {
            members,
            inert,
            domains,
        }
    }
}

/// The declared roots plus everything they transitively depend on,
/// workspace members only.
pub(crate) fn dependency_closure(
    roots: &[String],
    dependencies: &BTreeMap<String, BTreeSet<String>>,
) -> BTreeSet<String> {
    let mut closure: BTreeSet<String> = BTreeSet::new();
    let mut frontier: Vec<String> = roots.to_vec();
    while let Some(name) = frontier.pop() {
        if !closure.insert(name.clone()) {
            continue;
        }
        if let Some(declared) = dependencies.get(&name) {
            frontier.extend(declared.iter().cloned());
        }
    }
    closure
}

fn relative_to(path: &std::path::Path, workspace_root: &str) -> PathBuf {
    path.strip_prefix(workspace_root)
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|_| path.to_path_buf())
}

fn string_at(value: &serde_json::Value, key: &'static str) -> Result<String, XtaskError> {
    value
        .get(key)
        .and_then(|field| field.as_str())
        .map(str::to_owned)
        .ok_or_else(|| shape_error(key))
}

fn string_list(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(|list| list.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|entry| entry.as_str())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn shape_error(field: &'static str) -> XtaskError {
    XtaskError::Usage {
        message: format!("cargo metadata is missing {field} for the change classification"),
    }
}
