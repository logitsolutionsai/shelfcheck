//! The data files this crate embeds are copies of the workspace's own
//! (scripts/sync-crate-data.sh), so that `cargo package` includes them.
//! This test fails while a copy differs from the workspace file. A packaged
//! crate has no workspace around it, and then the test has nothing to compare.

use std::fs;
use std::path::{Path, PathBuf};

fn files(dir: &Path, base: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read_dir") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            files(&path, base, out);
        } else {
            out.push(path.strip_prefix(base).expect("prefix").to_path_buf());
        }
    }
}

fn same_tree(workspace: &Path, copy: &Path) {
    let (mut a, mut b) = (Vec::new(), Vec::new());
    files(workspace, workspace, &mut a);
    files(copy, copy, &mut b);
    a.sort();
    b.sort();
    assert_eq!(
        a,
        b,
        "{} and {} list different files; run scripts/sync-crate-data.sh",
        workspace.display(),
        copy.display()
    );
    for f in &a {
        assert!(
            fs::read(workspace.join(f)).expect("read") == fs::read(copy.join(f)).expect("read"),
            "{} differs from the workspace copy; run scripts/sync-crate-data.sh",
            copy.join(f).display()
        );
    }
}

#[test]
fn crate_data_matches_workspace() {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = here.join("../..");
    if !root.join("Cargo.toml").exists() || !root.join("crates").exists() {
        return;
    }
    same_tree(&here.join("../../assets/fonts"), &here.join("assets/fonts"));
    for f in ["LICENSE", "NOTICE"] {
        assert!(
            fs::read(root.join(f)).expect("read") == fs::read(here.join(f)).expect("read"),
            "{f} differs from the workspace copy; run scripts/sync-crate-data.sh"
        );
    }
}
