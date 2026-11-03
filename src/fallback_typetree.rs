use crate::serializedfile::TypeTreeNode;
use crate::unity_version::UnityVersion;
use std::collections::HashMap;
use std::sync::Mutex;
use once_cell::sync::Lazy;
use std::sync::Arc;
use crate::typetree_db::{StaticTypeTreeNode, TPK_DATABASE};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VersionRange {
    pub min_major: u16,
    pub min_minor: u16,
    pub max_major: u16,
    pub max_minor: u16,
}

impl VersionRange {
    pub const fn all() -> Self {
        Self { min_major: 0, min_minor: 0, max_major: u16::MAX, max_minor: u16::MAX }
    }

    pub fn matches(&self, version: &UnityVersion) -> bool {
        let val = version_value(version);
        val >= self.min_value() && val <= self.max_value()
    }

    pub fn min_value(&self) -> u32 {
        ((self.min_major as u32) << 16) | (self.min_minor as u32)
    }

    pub fn max_value(&self) -> u32 {
        ((self.max_major as u32) << 16) | (self.max_minor as u32)
    }
}

fn version_value(version: &UnityVersion) -> u32 {
    ((version.major() as u32) << 16) | (version.minor() as u32)
}

static CACHE: Lazy<Mutex<HashMap<(i32, u16, u16), Arc<[TypeTreeNode]>>>> = Lazy::new(|| Mutex::new(HashMap::new()));

pub fn get_fallback_nodes(class_id: i32, unity_version: &UnityVersion) -> Arc<[TypeTreeNode]> {
    let key = (class_id, unity_version.major(), unity_version.minor());
    let mut cache = CACHE.lock().unwrap();
    if let Some(nodes) = cache.get(&key) {
        return nodes.clone();
    }
    let nodes: Arc<[TypeTreeNode]> = get_fallback_nodes_uncached(class_id, unity_version).into();
    cache.insert(key, nodes.clone());
    nodes
}

fn get_fallback_nodes_uncached(class_id: i32, unity_version: &UnityVersion) -> Vec<TypeTreeNode> {
    let val = version_value(unity_version);
    let mut oldest: Option<&'static [StaticTypeTreeNode]> = None;
    let mut nearest: Option<(u32, &'static [StaticTypeTreeNode])> = None;
    for entry in TPK_DATABASE {
        if entry.class_id != class_id {
            continue;
        }
        if entry.version_range.matches(unity_version) {
            return build_nodes(entry.nodes);
        }
        if oldest.is_none() {
            oldest = Some(entry.nodes);
        }
        let min_val = entry.version_range.min_value();
        if min_val <= val && nearest.map_or(true, |(best, _)| min_val > best) {
            nearest = Some((min_val, entry.nodes));
        }
    }
    match nearest.map(|(_, nodes)| nodes).or(oldest) {
        Some(nodes) => build_nodes(nodes),
        None => Vec::new(),
    }
}

fn build_nodes(nodes: &'static [StaticTypeTreeNode]) -> Vec<TypeTreeNode> {
    nodes.iter().enumerate().map(|(idx, n)| n.to_type_tree_node(idx as i32)).collect()
}
