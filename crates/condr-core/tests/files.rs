use std::fs;
use std::path::{Path, PathBuf};

use condr_core::{FileContent, FileKind, MAX_FILE_BYTES, list_directory, read_file};

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("condr-files-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn listing_puts_directories_first_hides_git_and_refuses_escapes() {
    let root = scratch("listing");
    fs::create_dir_all(root.join("src/app")).unwrap();
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::create_dir_all(root.join("Docs")).unwrap();
    fs::write(root.join("b.txt"), "b").unwrap();
    fs::write(root.join("A.txt"), "a").unwrap();
    fs::write(root.join("src/main.rs"), "fn main() {}").unwrap();

    let listing = list_directory(&root, Path::new("")).unwrap();
    let names: Vec<(&str, FileKind)> = listing
        .entries
        .iter()
        .map(|entry| (entry.name.as_str(), entry.kind))
        .collect();
    assert_eq!(
        names,
        [
            ("Docs", FileKind::Directory),
            ("src", FileKind::Directory),
            ("A.txt", FileKind::File),
            ("b.txt", FileKind::File),
        ]
    );
    assert!(!listing.truncated);

    let src = list_directory(&root, Path::new("src")).unwrap();
    assert_eq!(src.entries[0].name, "app");
    assert_eq!(src.entries[1].name, "main.rs");

    assert!(list_directory(&root, Path::new("../")).is_err());
    assert!(list_directory(&root, Path::new("missing")).is_err());

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn reading_tells_text_binary_and_oversized_files_apart() {
    let root = scratch("reading");
    fs::write(root.join("notes.txt"), "alpha\nbeta\n").unwrap();
    fs::write(root.join("image.bin"), [0x89, b'P', b'N', b'G', 0, 1, 2]).unwrap();
    fs::write(
        root.join("huge.txt"),
        vec![b'x'; MAX_FILE_BYTES as usize + 1],
    )
    .unwrap();
    fs::create_dir_all(root.join("dir")).unwrap();

    assert_eq!(
        read_file(&root, Path::new("notes.txt")).unwrap(),
        FileContent::Text {
            text: "alpha\nbeta\n".into()
        }
    );
    assert_eq!(
        read_file(&root, Path::new("image.bin")).unwrap(),
        FileContent::Binary
    );
    assert!(matches!(
        read_file(&root, Path::new("huge.txt")).unwrap(),
        FileContent::TooLarge { bytes } if bytes == MAX_FILE_BYTES + 1
    ));
    assert!(read_file(&root, Path::new("dir")).is_err());
    assert!(read_file(&root, Path::new("")).is_err());
    assert!(read_file(&root, Path::new("../notes.txt")).is_err());

    let _ = fs::remove_dir_all(&root);
}
