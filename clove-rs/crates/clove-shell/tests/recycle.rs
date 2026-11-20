//! The one test that touches the real Recycle Bin.
//!
//! It is deliberately self-contained and reversible: a uniquely named temp file is
//! recycled, its receipt is checked, and it is put back. If `find_trashed` ever regresses,
//! the item is still safely in the Recycle Bin — nothing is lost.

use std::path::PathBuf;

use clove_shell::{is_direct_child, put_back, trash};

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "clove-shell-recycle-{}-{tag}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_file_goes_to_the_recycle_bin_and_comes_back() {
    let root = temp_dir("roundtrip");
    let file = root.join("clove-roundtrip-proof.txt");
    let contents = b"clove puts this back";
    std::fs::write(&file, contents).unwrap();
    assert!(is_direct_child(&file, &root));

    let outcome = trash(&file, &root, false);
    assert_eq!(outcome.failures, 0, "the shell refused to recycle");
    assert_eq!(outcome.moves.len(), 1);
    assert!(!file.exists(), "the original must be gone");

    let mv = &outcome.moves[0];
    assert_eq!(mv.original, std::path::absolute(&file).unwrap());
    let trashed = mv
        .trashed
        .as_ref()
        .expect("receipt should locate the $R payload");
    assert!(trashed.exists(), "receipt points at {}", trashed.display());
    assert_eq!(
        std::fs::read(trashed).unwrap(),
        contents,
        "the recycled copy is intact"
    );

    let restored = put_back(std::slice::from_ref(mv));
    assert_eq!(restored.restored, 1);
    assert_eq!(restored.missing, 0);
    assert_eq!(std::fs::read(&file).unwrap(), contents);

    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn emptying_a_folder_recycles_its_children_and_keeps_the_folder() {
    let root = temp_dir("emptying");
    let folder = root.join("cache");
    std::fs::create_dir_all(folder.join("nested")).unwrap();
    std::fs::write(folder.join("a.bin"), b"a").unwrap();
    std::fs::write(folder.join("nested").join("b.bin"), b"b").unwrap();

    let outcome = trash(&folder, &root, true);
    assert_eq!(outcome.failures, 0, "every child should recycle");
    assert_eq!(outcome.moves.len(), 2);
    assert!(folder.exists(), "the folder itself stays");
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0);

    let restored = put_back(&outcome.moves);
    assert_eq!(restored.restored, 2);
    assert_eq!(std::fs::read(folder.join("a.bin")).unwrap(), b"a");
    assert_eq!(std::fs::read(folder.join("nested").join("b.bin")).unwrap(), b"b");

    std::fs::remove_dir_all(&root).unwrap();
}
