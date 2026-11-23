//! Plugin dependency resolution.
//!
//! Builds a dependency graph over the discovered manifests, detects missing
//! requirements, version conflicts, and cycles, and returns plugins in
//! topological order (dependencies first) so loading never relies on
//! filesystem order.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use crate::error::{PluginError, PluginResult};
use crate::id::PluginId;
use crate::manifest::PluginDependency;
use crate::version::PluginVersion;

/// The resolved dependency ordering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyOrder {
    /// Ids in dependency-first load order.
    pub order: Vec<PluginId>,
    /// For every plugin, the ids that must be enabled before it.
    pub depends_on: BTreeMap<PluginId, BTreeSet<PluginId>>,
}

/// Resolution failure kinds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyError {
    /// A dependency id is not among the discovered plugins.
    Missing { plugin: PluginId, missing: PluginId },
    /// A dependency's installed version does not satisfy the requirement.
    Conflict {
        plugin: PluginId,
        dependency: PluginId,
        installed: PluginVersion,
        required: String,
    },
    /// The graph contains a cycle.
    Circular { cycle: Vec<PluginId> },
}

/// Resolves load order for the given manifests.
///
/// # Errors
/// [`DependencyError`] converted into [`PluginError::internal`] for missing
/// dependencies, version conflicts, and circular dependencies.
pub fn resolve(
    manifests: impl IntoIterator<Item = (PluginId, PluginVersion, Vec<PluginDependency>)>,
) -> PluginResult<DependencyOrder> {
    let inputs: Vec<(PluginId, PluginVersion, Vec<PluginDependency>)> =
        manifests.into_iter().collect();

    let mut installed: HashMap<PluginId, PluginVersion> = HashMap::new();
    let mut requirements: HashMap<PluginId, Vec<PluginDependency>> = HashMap::new();
    for (id, version, deps) in &inputs {
        installed.insert(id.clone(), *version);
        requirements.insert(id.clone(), deps.clone());
    }

    for (id, _version, deps) in &inputs {
        for dependency in deps {
            let Some(installed_version) = installed.get(&dependency.id) else {
                let err = DependencyError::Missing {
                    plugin: id.clone(),
                    missing: dependency.id.clone(),
                };
                return Err(PluginError::internal(format!("{err}")));
            };
            if !dependency.version_req.matches(*installed_version) {
                let err = DependencyError::Conflict {
                    plugin: id.clone(),
                    dependency: dependency.id.clone(),
                    installed: *installed_version,
                    required: dependency.version_req.to_string(),
                };
                return Err(PluginError::internal(format!("{err}")));
            }
        }
    }

    // Kahn's algorithm over "depends on" edges.
    let mut edges: HashMap<PluginId, BTreeSet<PluginId>> = HashMap::new();
    for (id, _, deps) in &inputs {
        edges.insert(id.clone(), deps.iter().map(|d| d.id.clone()).collect());
    }

    let mut indegree: HashMap<&PluginId, usize> = HashMap::new();
    let mut dependents: HashMap<&PluginId, Vec<&PluginId>> = HashMap::new();
    for (id, deps) in &edges {
        indegree.insert(id, deps.len());
        for dep in deps {
            dependents.entry(dep).or_default().push(id);
        }
    }

    let mut ready: VecDeque<&PluginId> = indegree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(id, _)| *id)
        .collect();
    let mut order: Vec<PluginId> = Vec::with_capacity(inputs.len());
    while let Some(id) = ready.pop_front() {
        order.push(id.clone());
        for dependent in dependents.get(id).map_or(&[] as &[_], Vec::as_slice) {
            let degree = indegree.get_mut(dependent).expect("edge recorded");
            *degree -= 1;
            if *degree == 0 {
                ready.push_back(dependent);
            }
        }
    }

    if order.len() != inputs.len() {
        let remaining: Vec<PluginId> = inputs
            .iter()
            .map(|(id, _, _)| id.clone())
            .filter(|id| !order.contains(id))
            .collect();
        let err = DependencyError::Circular { cycle: remaining };
        return Err(PluginError::internal(format!("{err}")));
    }

    let depends_on = edges
        .into_iter()
        .collect::<BTreeMap<PluginId, BTreeSet<PluginId>>>();

    Ok(DependencyOrder { order, depends_on })
}

impl std::fmt::Display for DependencyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing { plugin, missing } => {
                write!(f, "{plugin} missing dependency {missing}")
            }
            Self::Conflict {
                plugin,
                dependency,
                installed,
                required,
            } => write!(
                f,
                "{plugin} requires {dependency} {required}, installed {installed}"
            ),
            Self::Circular { cycle } => {
                let names: Vec<&str> = cycle.iter().map(PluginId::as_str).collect();
                write!(f, "circular dependency: {}", names.join(" -> "))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version::VersionReq;

    fn dep(id: &str, req: &str) -> PluginDependency {
        PluginDependency {
            id: PluginId::new(id).unwrap(),
            version_req: VersionReq::parse(req).unwrap(),
        }
    }

    #[test]
    fn orders_dependencies_first() {
        let a = PluginId::new("org.nexus.rust").unwrap();
        let b = PluginId::new("org.nexus.debugger").unwrap();
        let c = PluginId::new("org.nexus.theme").unwrap();
        let result = resolve([
            (a.clone(), PluginVersion::new(1, 0, 0), vec![]),
            (
                b.clone(),
                PluginVersion::new(1, 0, 0),
                vec![dep("org.nexus.rust", ">= 1.0")],
            ),
            (c, PluginVersion::new(0, 2, 0), vec![]),
        ])
        .unwrap();
        let index_of = |id: &PluginId| result.order.iter().position(|x| x == id).unwrap();
        assert!(index_of(&a) < index_of(&b));
        assert!(result.depends_on.get(&b).unwrap().contains(&a));
    }

    #[test]
    fn detects_missing_dependency() {
        let a = PluginId::new("org.nexus.debugger").unwrap();
        let err = resolve([(
            a,
            PluginVersion::new(1, 0, 0),
            vec![dep("org.nexus.rust", ">= 1")],
        )])
        .unwrap_err();
        assert!(err.message.contains("missing dependency"));
    }

    #[test]
    fn detects_version_conflict() {
        let a = PluginId::new("org.nexus.rust").unwrap();
        let b = PluginId::new("org.nexus.debugger").unwrap();
        let err = resolve([
            (a, PluginVersion::new(1, 0, 0), vec![]),
            (
                b,
                PluginVersion::new(1, 0, 0),
                vec![dep("org.nexus.rust", ">= 1.2")],
            ),
        ])
        .unwrap_err();
        assert!(err.message.contains("requires org.nexus.rust >=1.2.0"));
    }

    #[test]
    fn detects_circular_dependency() {
        let a = PluginId::new("org.nexus.a").unwrap();
        let b = PluginId::new("org.nexus.b").unwrap();
        let err = resolve([
            (
                a.clone(),
                PluginVersion::new(1, 0, 0),
                vec![dep("org.nexus.b", "*")],
            ),
            (
                b.clone(),
                PluginVersion::new(1, 0, 0),
                vec![dep("org.nexus.a", "*")],
            ),
        ])
        .unwrap_err();
        assert!(err.message.contains("circular"));
    }

    #[test]
    fn empty_set_is_trivially_resolved() {
        let result = resolve(std::iter::empty()).unwrap();
        assert!(result.order.is_empty());
    }
}
