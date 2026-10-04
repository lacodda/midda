//! What the integration tests share: the one definition of "the same tree".

use midda_core::Tree;

/// Asserts two trees are the same, node for node, naming the first place they
/// differ by path rather than by index.
///
/// The same arena, not only the same totals: the node order decides which
/// name keeps shared bytes, so two trees that agree on every total but list
/// their nodes differently would still charge a hard-linked file to different
/// folders.
pub fn assert_same(expected: &Tree, actual: &Tree) {
    assert_eq!(expected.root_path(), actual.root_path());
    assert_eq!(expected.cluster_bytes(), actual.cluster_bytes(), "cluster size");
    assert!(expected.skipped().is_empty(), "the first tree skipped {:?}", expected.skipped());
    assert!(actual.skipped().is_empty(), "the second tree skipped {:?}", actual.skipped());

    let paths_expected: Vec<_> = expected.nodes().map(|(id, _)| expected.path_of(id)).collect();
    let paths_actual: Vec<_> = actual.nodes().map(|(id, _)| actual.path_of(id)).collect();
    assert_eq!(
        paths_expected, paths_actual,
        "the two trees hold different entries, or list them in a different order"
    );

    for ((id, a), (_, b)) in expected.nodes().zip(actual.nodes()) {
        let path = expected.path_of(id);
        let at = path.display();
        assert_eq!(a.kind, b.kind, "kind of {at}");
        assert_eq!(a.parent, b.parent, "parent of {at}");
        assert_eq!(a.children, b.children, "children of {at}");
        assert_eq!(a.size, b.size, "size of {at}");
        assert_eq!(a.traits, b.traits, "traits of {at}");
        assert_eq!(a.links, b.links, "link count of {at}");
        assert_eq!(a.identity, b.identity, "identity of {at}");
        assert_eq!(a.entries, b.entries, "entries under {at}");
        assert_eq!(a.modified, b.modified, "modified time of {at}");
        assert_eq!(a.subtree_modified, b.subtree_modified, "subtree modified time of {at}");
    }
    assert_eq!(expected.shared(), actual.shared(), "shared bytes");
}
