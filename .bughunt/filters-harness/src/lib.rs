include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../crates/squarebob-core/src/lib.rs"
));

extern crate self as squarebob_core;

mod app {
    pub mod helpers {
        pub fn fmt_size(bytes: u64) -> String {
            format!("{bytes} B")
        }
    }
}

mod exclusions {
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};

    #[derive(Default)]
    pub struct Exclusions {
        paths: HashSet<PathBuf>,
    }

    impl Exclusions {
        pub fn contains(&self, path: &Path) -> bool {
            self.paths.contains(path)
        }
    }
}

#[path = "../../../src/app/filters.rs"]
mod filters;

#[cfg(test)]
mod depth_tests {
    use super::filters::{collect_matching_paths, filter_tree};
    use super::DirEntry;
    use std::collections::HashSet;
    use std::path::PathBuf;

    fn deep_tree(depth: usize) -> DirEntry {
        let mut node = DirEntry::new_file(
            "needle.txt".to_string(),
            PathBuf::from("needle.txt"),
            1,
            "txt".to_string(),
            None,
        );
        for index in (0..depth).rev() {
            let mut parent =
                DirEntry::new_dir(format!("dir-{index}"), PathBuf::from(format!("dir-{index}")));
            parent.size = node.size;
            parent.file_count = node.file_count;
            parent.dir_count = node.dir_count + u64::from(node.is_dir);
            parent.children.push(node);
            node = parent;
        }
        node
    }

    #[test]
    fn filters_and_search_handle_deep_tree() {
        const DEPTH: usize = 20_000;
        let tree = deep_tree(DEPTH);
        let filtered = filter_tree(&tree, 1, 1, false);
        assert_eq!(filtered.iter().count(), DEPTH + 1);

        let mut matching = HashSet::new();
        assert!(collect_matching_paths(
            &tree,
            "needle",
            &[],
            &mut matching
        ));
        assert_eq!(matching.len(), DEPTH + 1);
    }
}
