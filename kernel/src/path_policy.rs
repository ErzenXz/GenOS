//! Canonical bounded pathname policy shared by VFS and syscall validation.
//! This module has no mutable state, hardware access, or presentation effects.
//! Absolute paths use nonempty ASCII components; dot navigation and duplicate
//! separators are rejected instead of being interpreted differently by callers.
use crate::console_text::FixedText;
use genos_abi::{USER_PATH_MAX, USER_WRITABLE_PREFIX};

pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= USER_PATH_MAX
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

pub fn valid_absolute_path(path: &str) -> bool {
    path.len() <= USER_PATH_MAX
        && (path == "/"
            || path
                .strip_prefix('/')
                .is_some_and(|rest| rest.split('/').all(valid_name)))
}

pub fn is_user_writable_path(path: &str) -> bool {
    valid_absolute_path(path)
        && path.len() > USER_WRITABLE_PREFIX.len()
        && path
            .get(..USER_WRITABLE_PREFIX.len())
            .is_some_and(|prefix| paths_equal(prefix, USER_WRITABLE_PREFIX))
}

pub fn is_user_writable_directory(path: &str) -> bool {
    paths_equal(path, USER_WRITABLE_PREFIX.trim_end_matches('/')) || is_user_writable_path(path)
}

pub fn paths_equal(left: &str, right: &str) -> bool {
    left.len() == right.len()
        && left
            .bytes()
            .zip(right.bytes())
            .all(|(left, right)| left.eq_ignore_ascii_case(&right))
}

pub fn join_child_path(parent: FixedText, name: FixedText) -> Option<FixedText> {
    let name = name.as_str();
    if !valid_absolute_path(parent.as_str()) || !valid_name(name) {
        return None;
    }
    let separator = if parent.as_str() == "/" { "" } else { "/" };
    let length = parent
        .len()
        .checked_add(separator.len())?
        .checked_add(name.len())?;
    if length > USER_PATH_MAX {
        return None;
    }
    let mut target = parent;
    target.push_str(separator);
    target.push_str(name);
    (target.len() == length).then_some(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ambiguous_and_noncanonical_paths_are_rejected_before_authority() {
        for path in [
            "",
            "USER/A",
            "//USER/A",
            "/USER/",
            "/USER/./A",
            "/USER/../A",
            "/USER//A",
            "/USER/A B",
            "/USER/é",
            "/USER/..",
        ] {
            assert!(!valid_absolute_path(path), "{path}");
            assert!(!is_user_writable_path(path), "{path}");
        }
        for path in ["/", "/USER", "/user/.hidden", "/USER/A-B_1.TXT"] {
            assert!(valid_absolute_path(path));
        }
        assert!(!is_user_writable_path("/USERLAND/A"));
        assert!(!is_user_writable_path("/MEMORY.STATUS"));
        assert!(!is_user_writable_path("/USER"));
    }

    #[test]
    fn vfs_uses_the_same_creation_policy_without_partial_insertion() {
        let mut vfs = crate::vfs::RamVfs::new();
        vfs.init_root();
        vfs.mkdir("/USER").unwrap();
        let count = vfs.count();
        for path in ["/USER/..", "/USER/.", "/USER//X", "/USER/", "/USER/A B"] {
            assert_eq!(vfs.touch(path), Err(crate::vfs::VfsError::InvalidPath));
            assert_eq!(vfs.count(), count);
        }
        vfs.touch("/USER/GOOD.TXT").unwrap();
        assert!(vfs.find("/user/good.txt").is_some());
    }

    #[test]
    fn namespace_children_stay_beneath_the_owned_directory() {
        let parent = FixedText::from_str("/USER/PROJECTS");
        assert_eq!(
            join_child_path(parent, FixedText::from_str("GENOS"))
                .expect("valid child")
                .as_str(),
            "/USER/PROJECTS/GENOS"
        );
        assert!(join_child_path(parent, FixedText::from_str("..")).is_none());
        assert!(join_child_path(parent, FixedText::from_str("nested/name")).is_none());
        assert!(paths_equal("/USER/NOTE.TXT", "/user/note.txt"));
        assert!(is_user_writable_path("/user/note.txt"));
        assert!(is_user_writable_directory("/user"));
        assert!(join_child_path(
            FixedText::from_str("/USER/ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789ABCDEFGHIJKLMNOPQRST"),
            FixedText::from_str("TOO-LONG")
        )
        .is_none());
    }
}
