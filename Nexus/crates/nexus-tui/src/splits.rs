//! Editor split tree.
//!
//! Splits are pure structure over editor groups and are fully independent of
//! the document model: a group can be empty, show any document, or hold a
//! placeholder. This keeps multi-editor layouts composable with tabs.

use nexus_core::ids::FileId;

/// Split orientation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDirection {
    Vertical,
    Horizontal,
}

/// Identifier of an editor group (leaf).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GroupId(pub usize);

impl GroupId {
    #[must_use]
    pub const fn first() -> Self {
        Self(0)
    }
}

/// A group currently showing one document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorGroup {
    pub id: GroupId,
    pub document: Option<FileId>,
}

impl EditorGroup {
    #[must_use]
    pub fn new(id: GroupId) -> Self {
        Self { id, document: None }
    }
}

/// Recursive split tree over groups.
#[derive(Debug, Clone)]
pub struct SplitTree {
    pub root: SplitNode,
    next_group: usize,
}

/// A node: either a group (leaf) or a directional split of two subtrees.
#[derive(Debug, Clone)]
pub enum SplitNode {
    Group(EditorGroup),
    Split {
        direction: SplitDirection,
        first: Box<SplitNode>,
        second: Box<SplitNode>,
    },
}

impl Default for SplitTree {
    fn default() -> Self {
        Self::single()
    }
}

impl SplitTree {
    /// A single, empty group.
    #[must_use]
    pub fn single() -> Self {
        Self {
            root: SplitNode::Group(EditorGroup::new(GroupId::first())),
            next_group: 1,
        }
    }

    /// Splits the group at `path` (empty => root) in `direction`.
    /// Returns the new group's id.
    pub fn split(&mut self, path: &[usize], direction: SplitDirection) -> Option<GroupId> {
        let new_id = GroupId(self.next_group);
        let second_id = GroupId(self.next_group);
        self.next_group += 1;
        self.walk_mut(path, &mut move |group| SplitNode::Split {
            direction,
            first: Box::new(SplitNode::Group(group.clone())),
            second: Box::new(SplitNode::Group(EditorGroup::new(second_id))),
        })
        .map(|_| new_id)
    }

    /// Removes the split at `path` (empty path collapses the whole tree),
    /// merging that subtree into a single group.
    pub fn close_at(&mut self, path: &[usize]) {
        collapse_walk(&mut self.root, path, 0);
    }

    /// Walks to the node at `path`, applying `f` and remembering it changed.
    fn walk_mut(
        &mut self,
        path: &[usize],
        f: &mut dyn FnMut(&EditorGroup) -> SplitNode,
    ) -> Option<()> {
        replace_walk(&mut self.root, path, 0, f)
    }

    /// The document open in the group at `path`, when the path names a group.
    #[must_use]
    pub fn document_at(&self, path: &[usize]) -> Option<FileId> {
        match get_walk(&self.root, path, 0) {
            Some(SplitNode::Group(g)) => g.document,
            _ => None,
        }
    }

    /// Opens `doc` in the group at `path` (empty path => first group).
    pub fn open_at(&mut self, path: &[usize], doc: FileId) {
        if let Some(SplitNode::Group(g)) = get_mut_walk(&mut self.root, path, 0) {
            g.document = Some(doc);
        }
    }

    /// The groups in depth-first order.
    pub fn groups(&self) -> Vec<&EditorGroup> {
        let mut out = Vec::new();
        collect_groups(&self.root, &mut out);
        out
    }

    #[must_use]
    pub fn group_count(&self) -> usize {
        self.groups().len()
    }
}

fn replace_walk(
    node: &mut SplitNode,
    path: &[usize],
    depth: usize,
    f: &mut dyn FnMut(&EditorGroup) -> SplitNode,
) -> Option<()> {
    match node {
        SplitNode::Group(group) if path.len() == depth => {
            *node = f(group);
            Some(())
        }
        SplitNode::Group(_) => None,
        SplitNode::Split { first, second, .. } => {
            let next = path.get(depth)?;
            if *next == 0 {
                replace_walk(first, path, depth + 1, f)
            } else if *next == 1 {
                replace_walk(second, path, depth + 1, f)
            } else {
                None
            }
        }
    }
}

fn get_walk<'a>(node: &'a SplitNode, path: &[usize], depth: usize) -> Option<&'a SplitNode> {
    match node {
        SplitNode::Group(_) if path.len() == depth => Some(node),
        SplitNode::Group(_) => None,
        SplitNode::Split { first, second, .. } => {
            let next = path.get(depth)?;
            if *next == 0 {
                get_walk(first, path, depth + 1)
            } else if *next == 1 {
                get_walk(second, path, depth + 1)
            } else {
                None
            }
        }
    }
}

fn get_mut_walk<'a>(
    node: &'a mut SplitNode,
    path: &[usize],
    depth: usize,
) -> Option<&'a mut SplitNode> {
    match node {
        SplitNode::Group(_) if path.len() == depth => Some(node),
        SplitNode::Group(_) => None,
        SplitNode::Split { first, second, .. } => {
            let next = path.get(depth)?;
            if *next == 0 {
                get_mut_walk(first, path, depth + 1)
            } else if *next == 1 {
                get_mut_walk(second, path, depth + 1)
            } else {
                None
            }
        }
    }
}

fn collect_groups<'a>(node: &'a SplitNode, out: &mut Vec<&'a EditorGroup>) {
    match node {
        SplitNode::Group(group) => out.push(group),
        SplitNode::Split { first, second, .. } => {
            collect_groups(first, out);
            collect_groups(second, out);
        }
    }
}

/// Collapses the subtree at `path` into a single group (its leftmost leaf).
fn collapse_walk(node: &mut SplitNode, path: &[usize], depth: usize) {
    if path.len() == depth {
        if let SplitNode::Split { first, .. } = node {
            *node = leftmost_group(first);
        }
        return;
    }
    if let SplitNode::Split { first, second, .. } = node {
        match path[depth] {
            0 => collapse_walk(first, path, depth + 1),
            1 => collapse_walk(second, path, depth + 1),
            _ => {}
        }
    }
}

fn leftmost_group(node: &SplitNode) -> SplitNode {
    match node {
        SplitNode::Group(_) => node.clone(),
        SplitNode::Split { first, .. } => leftmost_group(first),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fid(n: u8) -> FileId {
        FileId::from_u64(n as u64)
    }

    #[test]
    fn single_tree_has_one_group() {
        let tree = SplitTree::single();
        assert_eq!(tree.group_count(), 1);
        assert_eq!(tree.groups()[0].id, GroupId::first());
    }

    #[test]
    fn split_adds_a_group_and_root_becomes_a_split() {
        let mut tree = SplitTree::single();
        let new = tree.split(&[], SplitDirection::Horizontal).unwrap();
        assert_eq!(tree.group_count(), 2);
        assert_ne!(new, GroupId::first());
        match &tree.root {
            SplitNode::Split { direction, .. } => {
                assert_eq!(*direction, SplitDirection::Horizontal);
            }
            _ => panic!("expected split root"),
        }
    }

    #[test]
    fn nested_split_and_document_flow() {
        let mut tree = SplitTree::single();
        tree.open_at(&[], fid(1));
        tree.split(&[], SplitDirection::Vertical);
        let in_left = tree.split(&[0], SplitDirection::Vertical).unwrap();
        tree.open_at(&[0, 0], fid(2));
        tree.open_at(&[0, 1], fid(3));
        assert_eq!(tree.document_at(&[0, 0]), Some(fid(2)));
        assert_eq!(tree.document_at(&[0, 1]), Some(fid(3)));
        assert!(in_left.0 > 0);
        assert_eq!(tree.group_count(), 3);
    }

    #[test]
    fn closing_merges_to_single_group() {
        let mut tree = SplitTree::single();
        tree.split(&[], SplitDirection::Vertical);
        assert_eq!(tree.group_count(), 2);
        tree.close_at(&[]);
        assert_eq!(tree.group_count(), 1);
    }

    #[test]
    fn invalid_paths_are_safe() {
        let mut tree = SplitTree::single();
        assert!(tree.split(&[3, 3], SplitDirection::Vertical).is_none());
        assert_eq!(tree.document_at(&[5]), None);
        tree.open_at(&[1, 1, 1], fid(9)); // no-op, no panic
        assert_eq!(tree.group_count(), 1);
    }
}
