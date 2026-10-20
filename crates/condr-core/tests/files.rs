use std::fs;
use std::path::{Path, PathBuf};

use relative_path::RelativePath;

use condr_core::{
    FileContent, FileKind, FileMutation, FileMutationResult, MAX_FILE_BYTES, MAX_IMAGE_BYTES,
    browse_directory, file_version, list_directory, mutate_file, read_file,
};

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

    let listing = list_directory(&root, RelativePath::new("")).unwrap();
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

    let src = list_directory(&root, RelativePath::new("src")).unwrap();
    assert_eq!(src.entries[0].name, "app");
    assert_eq!(src.entries[1].name, "main.rs");

    assert!(list_directory(&root, RelativePath::new("../")).is_err());
    assert!(list_directory(&root, RelativePath::new("missing")).is_err());

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn browsing_lists_only_directories_names_the_parent_and_starts_at_home() {
    let root = scratch("browsing");
    fs::create_dir_all(root.join("project")).unwrap();
    // More files than a listing holds must not crowd out the directory.
    for index in 0..=condr_core::MAX_DIRECTORY_ENTRIES {
        fs::write(root.join(format!("file-{index}.txt")), "").unwrap();
    }

    let browsed = browse_directory(&root).unwrap();
    assert_eq!(browsed.path, root);
    assert_eq!(browsed.parent.as_deref(), root.parent());
    let names: Vec<&str> = browsed
        .listing
        .entries
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(names, ["project"]);
    assert!(!browsed.listing.truncated);

    let home = dirs::home_dir().unwrap();
    assert_eq!(browse_directory(Path::new("")).unwrap().path, home);
    let tilde = browse_directory(Path::new("~")).unwrap();
    assert_eq!(tilde.path, home, "a leading ~ is the home directory");
    assert_eq!(tilde.parent.as_deref(), home.parent());
    assert!(browse_directory(Path::new("relative")).is_err());
    assert!(browse_directory(&root.join("missing")).is_err());

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
        read_file(&root, RelativePath::new("notes.txt")).unwrap(),
        FileContent::Text {
            text: "alpha\nbeta\n".into(),
            version: Some(file_version(b"alpha\nbeta\n")),
            lossy: false,
        }
    );
    assert_eq!(
        read_file(&root, RelativePath::new("image.bin")).unwrap(),
        FileContent::Binary
    );
    assert!(matches!(
        read_file(&root, RelativePath::new("huge.txt")).unwrap(),
        FileContent::TooLarge { bytes } if bytes == MAX_FILE_BYTES + 1
    ));
    assert!(read_file(&root, RelativePath::new("dir")).is_err());
    assert!(read_file(&root, RelativePath::new("")).is_err());
    assert!(read_file(&root, RelativePath::new("../notes.txt")).is_err());

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn images_are_sent_as_bytes_up_to_their_own_limit() {
    let root = scratch("images");
    let png = [0x89, b'P', b'N', b'G', 0, 1, 2];
    fs::write(root.join("shot.PNG"), png).unwrap();
    // Over the text limit, under the image one.
    let large = vec![b'x'; MAX_FILE_BYTES as usize + 1];
    fs::write(root.join("large.gif"), &large).unwrap();
    fs::write(
        root.join("huge.jpg"),
        vec![b'x'; MAX_IMAGE_BYTES as usize + 1],
    )
    .unwrap();
    fs::write(root.join("logo.svg"), "<svg/>").unwrap();

    assert_eq!(
        read_file(&root, RelativePath::new("shot.PNG")).unwrap(),
        FileContent::Image {
            bytes: png.to_vec()
        }
    );
    assert_eq!(
        read_file(&root, RelativePath::new("large.gif")).unwrap(),
        FileContent::Image { bytes: large }
    );
    assert!(matches!(
        read_file(&root, RelativePath::new("huge.jpg")).unwrap(),
        FileContent::TooLarge { bytes } if bytes == MAX_IMAGE_BYTES + 1
    ));
    assert_eq!(
        read_file(&root, RelativePath::new("logo.svg")).unwrap(),
        FileContent::Text {
            text: "<svg/>".into(),
            version: Some(file_version(b"<svg/>")),
            lossy: false,
        },
        "SVG is text"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn only_exact_text_carries_a_version() {
    let root = scratch("versions");
    fs::write(root.join("invalid.txt"), b"caf\xe9").unwrap();
    fs::write(root.join("literal.txt"), "\u{FFFD}").unwrap();

    assert_eq!(
        read_file(&root, RelativePath::new("invalid.txt")).unwrap(),
        FileContent::Text {
            text: "caf\u{FFFD}".into(),
            version: None,
            lossy: true,
        }
    );
    assert!(matches!(
        read_file(&root, RelativePath::new("literal.txt")).unwrap(),
        FileContent::Text {
            version: Some(_),
            lossy: false,
            ..
        }
    ));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/dev/null", root.join("null")).unwrap();
        assert!(read_file(&root, RelativePath::new("null")).is_err());
    }

    let _ = fs::remove_dir_all(&root);
}

fn write(path: &str, text: &str, base: Option<u64>) -> FileMutation {
    FileMutation::Write {
        path: path.into(),
        text: text.into(),
        base,
    }
}

fn version_of(root: &Path, path: &str) -> Option<u64> {
    match read_file(root, RelativePath::new(path)).unwrap() {
        FileContent::Text { version, .. } => version,
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_write_needs_the_version_it_started_from() {
    let root = scratch("writes");
    fs::write(root.join("plan.md"), "one\r\n").unwrap();
    let base = version_of(&root, "plan.md");

    let saved = mutate_file(&root, &write("plan.md", "one\r\ntwo\r\n", base));
    assert_eq!(
        saved,
        FileMutationResult::Done {
            version: Some(file_version(b"one\r\ntwo\r\n"))
        }
    );
    assert_eq!(fs::read(root.join("plan.md")).unwrap(), b"one\r\ntwo\r\n");
    // The same base again: the file has moved on.
    assert_eq!(
        mutate_file(&root, &write("plan.md", "three", base)),
        FileMutationResult::Conflict
    );

    // A versioned save of a file that is gone conflicts; without a version it recreates it.
    fs::remove_file(root.join("plan.md")).unwrap();
    assert_eq!(
        mutate_file(&root, &write("plan.md", "back", base)),
        FileMutationResult::Conflict
    );
    assert!(matches!(
        mutate_file(&root, &write("plan.md", "back", None)),
        FileMutationResult::Done { version: Some(_) }
    ));
    assert_eq!(fs::read_to_string(root.join("plan.md")).unwrap(), "back");

    // Overwrite still passes every other check.
    fs::create_dir(root.join("dir")).unwrap();
    let huge = "x".repeat(MAX_FILE_BYTES as usize + 1);
    for refused in [
        write(".git/HEAD", "x", None),
        write("dir", "x", None),
        write("plan.md", &huge, None),
        write("../outside", "x", None),
    ] {
        assert!(
            matches!(mutate_file(&root, &refused), FileMutationResult::Invalid(_)),
            "{refused:?}"
        );
    }

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn writes_keep_symlinks_and_hard_links() {
    let root = scratch("links");
    fs::write(root.join("target.txt"), "old").unwrap();
    fs::hard_link(root.join("target.txt"), root.join("hard.txt")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("target.txt", root.join("soft.txt")).unwrap();

    let base = version_of(&root, "hard.txt");
    assert!(matches!(
        mutate_file(&root, &write("hard.txt", "new", base)),
        FileMutationResult::Done { .. }
    ));
    assert_eq!(fs::read_to_string(root.join("target.txt")).unwrap(), "new");
    #[cfg(unix)]
    {
        let base = version_of(&root, "soft.txt");
        assert!(matches!(
            mutate_file(&root, &write("soft.txt", "newer", base)),
            FileMutationResult::Done { .. }
        ));
        assert!(
            fs::symlink_metadata(root.join("soft.txt"))
                .unwrap()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(root.join("hard.txt")).unwrap(), "newer");
    }

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn two_saves_from_one_version_give_one_conflict() {
    let root = scratch("racing");
    fs::write(root.join("a.txt"), "start").unwrap();
    let base = version_of(&root, "a.txt");
    let results: Vec<FileMutationResult> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..2)
            .map(|n| {
                let root = &root;
                scope
                    .spawn(move || mutate_file(root, &write("a.txt", &format!("client {n}"), base)))
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect()
    });
    assert_eq!(
        results
            .iter()
            .filter(|result| **result == FileMutationResult::Conflict)
            .count(),
        1,
        "{results:?}"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn creating_never_replaces_and_refuses_git() {
    let root = scratch("creating");
    let create = |path: &str, kind| FileMutation::Create {
        path: path.into(),
        kind,
    };
    assert_eq!(
        mutate_file(&root, &create("notes.md", FileKind::File)),
        FileMutationResult::Done { version: None }
    );
    fs::write(root.join("notes.md"), "kept").unwrap();
    for refused in [
        create("notes.md", FileKind::File),
        create("notes.md", FileKind::Directory),
        create(".GIT", FileKind::Directory),
        create("sub/.git/config", FileKind::File),
    ] {
        assert!(
            matches!(mutate_file(&root, &refused), FileMutationResult::Invalid(_)),
            "{refused:?}"
        );
    }
    assert_eq!(fs::read_to_string(root.join("notes.md")).unwrap(), "kept");
    assert_eq!(
        mutate_file(&root, &create("docs", FileKind::Directory)),
        FileMutationResult::Done { version: None }
    );
    assert!(root.join("docs").is_dir());
    // A missing parent is not created.
    assert!(matches!(
        mutate_file(&root, &create("a/b.txt", FileKind::File)),
        FileMutationResult::Failed(_)
    ));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn renaming_never_lands_on_another_entry() {
    let root = scratch("renaming");
    let rename = |from: &str, to: &str| FileMutation::Rename {
        from: from.into(),
        to: to.into(),
    };
    fs::write(root.join("a.txt"), "a").unwrap();
    fs::write(root.join("b.txt"), "b").unwrap();
    fs::create_dir(root.join("dir")).unwrap();

    assert_eq!(
        mutate_file(&root, &rename("a.txt", "b.txt")),
        FileMutationResult::Invalid("already exists".into())
    );
    assert!(matches!(
        mutate_file(&root, &rename("a.txt", "dir/a.txt")),
        FileMutationResult::Invalid(_)
    ));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("missing", root.join("dangling")).unwrap();
        assert_eq!(
            mutate_file(&root, &rename("a.txt", "dangling")),
            FileMutationResult::Invalid("already exists".into())
        );
    }
    assert_eq!(fs::read_to_string(root.join("b.txt")).unwrap(), "b");

    // One entry may change only its case, whatever the volume; where the volume folds
    // case, a distinct entry differing in case alone is refused.
    assert_eq!(
        mutate_file(&root, &rename("a.txt", "A.txt")),
        FileMutationResult::Done { version: None }
    );
    let names: Vec<String> = fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(names.contains(&"A.txt".to_string()), "{names:?}");
    let folds_case = root.join("B.TXT").exists();
    let distinct = mutate_file(&root, &rename("A.txt", "B.TXT"));
    if folds_case {
        assert_eq!(
            distinct,
            FileMutationResult::Invalid("already exists".into())
        );
        assert_eq!(fs::read_to_string(root.join("b.txt")).unwrap(), "b");
    } else {
        assert_eq!(distinct, FileMutationResult::Done { version: None });
    }
    // So may a change of Unicode normalization alone, which APFS folds.
    fs::write(root.join("caf\u{e9}.md"), "c").unwrap();
    assert_eq!(
        mutate_file(&root, &rename("caf\u{e9}.md", "cafe\u{301}.md")),
        FileMutationResult::Done { version: None }
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn deleting_removes_links_not_what_they_point_at() {
    let root = scratch("deleting");
    let outside = scratch("deleting-outside");
    fs::write(outside.join("keep.txt"), "keep").unwrap();
    fs::create_dir_all(root.join("build/deep")).unwrap();
    fs::write(root.join("build/deep/out.o"), "o").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, root.join("build/link")).unwrap();
    let delete = |path: &str| FileMutation::Delete { path: path.into() };

    assert_eq!(
        mutate_file(&root, &delete("build")),
        FileMutationResult::Done { version: None }
    );
    assert!(!root.join("build").exists());
    assert!(outside.join("keep.txt").exists());
    assert!(matches!(
        mutate_file(&root, &delete("build")),
        FileMutationResult::Failed(_)
    ));
    assert!(matches!(
        mutate_file(&root, &delete(".git")),
        FileMutationResult::Invalid(_)
    ));

    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&outside);
}
