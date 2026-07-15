#[path = "../../../src/path_key.rs"]
pub mod path_key;

#[path = "../../../src/atomic_file.rs"]
pub mod atomic_file;

pub mod app {
    pub mod helpers {
        use squarebob_core::DirEntry;
        use std::collections::HashMap;

        pub fn compute_ext_stats(root: &DirEntry) -> Vec<(String, u64, u64)> {
            let mut stats = HashMap::<String, (u64, u64)>::new();
            for entry in root.iter().filter(|entry| !entry.is_dir) {
                let totals = stats.entry(entry.ext.clone()).or_default();
                totals.0 = totals.0.saturating_add(entry.size);
                totals.1 = totals.1.saturating_add(1);
            }
            stats
                .into_iter()
                .map(|(extension, (size, count))| (extension, size, count))
                .collect()
        }

        pub fn compute_size_range(root: &DirEntry) -> (u64, u64) {
            let mut sizes = root
                .iter()
                .filter(|entry| !entry.is_dir)
                .map(|entry| entry.size);
            let Some(first) = sizes.next() else {
                return (0, 0);
            };
            sizes.fold((first, first), |(min, max), size| {
                (min.min(size), max.max(size))
            })
        }
    }
}

#[path = "../../../src/cache.rs"]
pub mod cache;
