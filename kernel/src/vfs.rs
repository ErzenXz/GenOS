use crate::display::FixedText;

pub const MAX_NODES: usize = 32;
pub const MAX_FILE_BYTES: usize = 512;
pub const MAX_PATH_BYTES: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeKind {
    File,
    Directory,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VfsError {
    Exists,
    NotFound,
    NoSpace,
    IsDirectory,
    NotDirectory,
    InvalidPath,
    InvalidOffset,
}

#[derive(Clone, Copy)]
pub struct VfsNode {
    path: FixedText,
    kind: NodeKind,
    data: [u8; MAX_FILE_BYTES],
    len: usize,
}

impl VfsNode {
    pub const fn empty() -> Self {
        Self {
            path: FixedText::empty(),
            kind: NodeKind::File,
            data: [0; MAX_FILE_BYTES],
            len: 0,
        }
    }

    pub fn path(&self) -> &str {
        self.path.as_str()
    }

    pub fn kind(&self) -> NodeKind {
        self.kind
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn data(&self) -> &[u8] {
        &self.data[..self.len]
    }
}

pub struct RamVfs {
    nodes: [VfsNode; MAX_NODES],
    len: usize,
}

impl RamVfs {
    pub const fn new() -> Self {
        Self {
            nodes: [VfsNode::empty(); MAX_NODES],
            len: 0,
        }
    }

    pub fn init_root(&mut self) {
        let _ = self.mkdir("/");
    }

    pub fn seed_file(&mut self, name: &str, data: &[u8]) {
        let mut path = FixedText::from_str("/");
        path.push_str(name);
        let _ = self.write(path.as_str(), data);
    }

    pub fn mkdir(&mut self, path: &str) -> Result<(), VfsError> {
        self.insert(path, NodeKind::Directory, &[])
    }

    pub fn touch(&mut self, path: &str) -> Result<(), VfsError> {
        if self.find(path).is_some() {
            return Ok(());
        }
        self.insert(path, NodeKind::File, &[])
    }

    pub fn write(&mut self, path: &str, data: &[u8]) -> Result<(), VfsError> {
        if let Some(index) = self.find_index(path) {
            if self.nodes[index].kind == NodeKind::Directory {
                return Err(VfsError::IsDirectory);
            }
            self.nodes[index].len = 0;
            self.write_data(index, data)
        } else {
            self.insert(path, NodeKind::File, data)
        }
    }

    pub fn append(&mut self, path: &str, data: &[u8]) -> Result<(), VfsError> {
        let index = if let Some(index) = self.find_index(path) {
            index
        } else {
            self.insert(path, NodeKind::File, &[])?;
            self.find_index(path).ok_or(VfsError::NotFound)?
        };
        if self.nodes[index].kind == NodeKind::Directory {
            return Err(VfsError::IsDirectory);
        }
        let start = self.nodes[index].len;
        if start + data.len() > MAX_FILE_BYTES {
            return Err(VfsError::NoSpace);
        }
        for (offset, byte) in data.iter().enumerate() {
            self.nodes[index].data[start + offset] = *byte;
        }
        self.nodes[index].len += data.len();
        Ok(())
    }

    pub fn write_at(&mut self, path: &str, offset: usize, data: &[u8]) -> Result<usize, VfsError> {
        let index = self.find_index(path).ok_or(VfsError::NotFound)?;
        if self.nodes[index].kind == NodeKind::Directory {
            return Err(VfsError::IsDirectory);
        }
        if offset > self.nodes[index].len {
            return Err(VfsError::InvalidOffset);
        }
        let end = offset.checked_add(data.len()).ok_or(VfsError::NoSpace)?;
        if end > MAX_FILE_BYTES {
            return Err(VfsError::NoSpace);
        }
        self.nodes[index].data[offset..end].copy_from_slice(data);
        self.nodes[index].len = self.nodes[index].len.max(end);
        Ok(data.len())
    }

    pub fn read(&self, path: &str) -> Result<&[u8], VfsError> {
        let node = self.find(path).ok_or(VfsError::NotFound)?;
        if node.kind == NodeKind::Directory {
            return Err(VfsError::IsDirectory);
        }
        Ok(node.data())
    }

    pub fn remove(&mut self, path: &str) -> Result<(), VfsError> {
        let index = self.find_index(path).ok_or(VfsError::NotFound)?;
        if path == "/" {
            return Err(VfsError::InvalidPath);
        }
        let mut i = index + 1;
        while i < self.len {
            self.nodes[i - 1] = self.nodes[i];
            i += 1;
        }
        self.len -= 1;
        Ok(())
    }

    pub fn stat(&self, path: &str) -> Result<FixedText, VfsError> {
        let node = self.find(path).ok_or(VfsError::NotFound)?;
        let mut text = FixedText::from_str(node.path());
        text.push_str(" ");
        text.push_str(match node.kind {
            NodeKind::File => "file",
            NodeKind::Directory => "dir",
        });
        text.push_str(" bytes=");
        text.push_u64(node.len() as u64);
        Ok(text)
    }

    pub fn find(&self, path: &str) -> Option<&VfsNode> {
        self.nodes
            .iter()
            .take(self.len)
            .find(|node| eq_ignore_ascii_case(node.path(), path))
    }

    pub fn list_root(&self) -> VfsIter<'_> {
        VfsIter {
            vfs: self,
            index: 0,
        }
    }

    pub fn count(&self) -> usize {
        self.len
    }

    fn insert(&mut self, path: &str, kind: NodeKind, data: &[u8]) -> Result<(), VfsError> {
        if path.is_empty() || path.len() > MAX_PATH_BYTES || !path.starts_with('/') {
            return Err(VfsError::InvalidPath);
        }
        if self.find(path).is_some() {
            return Err(VfsError::Exists);
        }
        if self.len >= MAX_NODES {
            return Err(VfsError::NoSpace);
        }
        let index = self.len;
        self.nodes[index] = VfsNode {
            path: FixedText::from_str(path),
            kind,
            data: [0; MAX_FILE_BYTES],
            len: 0,
        };
        self.len += 1;
        if kind == NodeKind::File {
            self.write_data(index, data)?;
        }
        Ok(())
    }

    fn write_data(&mut self, index: usize, data: &[u8]) -> Result<(), VfsError> {
        if data.len() > MAX_FILE_BYTES {
            return Err(VfsError::NoSpace);
        }
        for (offset, byte) in data.iter().enumerate() {
            self.nodes[index].data[offset] = *byte;
        }
        self.nodes[index].len = data.len();
        Ok(())
    }

    fn find_index(&self, path: &str) -> Option<usize> {
        self.nodes
            .iter()
            .take(self.len)
            .position(|node| eq_ignore_ascii_case(node.path(), path))
    }
}

impl Default for RamVfs {
    fn default() -> Self {
        Self::new()
    }
}

pub struct VfsIter<'a> {
    vfs: &'a RamVfs,
    index: usize,
}

impl<'a> Iterator for VfsIter<'a> {
    type Item = &'a VfsNode;

    fn next(&mut self) -> Option<Self::Item> {
        while self.index < self.vfs.len {
            let node = &self.vfs.nodes[self.index];
            self.index += 1;
            if node.path() != "/" {
                return Some(node);
            }
        }
        None
    }
}

fn eq_ignore_ascii_case(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .all(|(left, right)| left.eq_ignore_ascii_case(&right))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vfs_create_write_read_delete() {
        let mut vfs = RamVfs::new();
        vfs.init_root();
        vfs.write("/hello.txt", b"hello").unwrap();
        vfs.append("/hello.txt", b" world").unwrap();
        assert_eq!(vfs.read("/HELLO.TXT").unwrap(), b"hello world");
        assert!(vfs
            .stat("/hello.txt")
            .unwrap()
            .as_str()
            .contains("bytes=11"));
        vfs.remove("/hello.txt").unwrap();
        assert_eq!(vfs.read("/hello.txt"), Err(VfsError::NotFound));
    }

    #[test]
    fn vfs_lists_seeded_files() {
        let mut vfs = RamVfs::new();
        vfs.init_root();
        vfs.seed_file("README.TXT", b"ok");
        assert_eq!(vfs.list_root().count(), 1);
    }

    #[test]
    fn offset_writes_overwrite_extend_and_reject_holes() {
        let mut vfs = RamVfs::new();
        vfs.init_root();
        vfs.write("/note.txt", b"hello").unwrap();
        assert_eq!(vfs.write_at("/note.txt", 2, b"YY"), Ok(2));
        assert_eq!(vfs.write_at("/note.txt", 5, b"!"), Ok(1));
        assert_eq!(vfs.read("/note.txt"), Ok(&b"heYYo!"[..]));
        assert_eq!(
            vfs.write_at("/note.txt", 7, b"gap"),
            Err(VfsError::InvalidOffset)
        );
    }
}
