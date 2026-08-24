use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::{StorageError, StorageResult};

/// A SHA-256 content digest.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ContentDigest(pub [u8; 32]);

/// Calculate the digest of exact object bytes.
#[must_use]
pub fn digest_bytes(bytes: &[u8]) -> ContentDigest {
    ContentDigest(Sha256::digest(bytes).into())
}

/// One file-system name component.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ObjectName(OsString);

impl ObjectName {
    /// Validate and copy one normal path component.
    pub fn new(name: impl AsRef<OsStr>) -> StorageResult<Self> {
        let name = name.as_ref();
        let mut components = Path::new(name).components();
        let is_normal =
            matches!(components.next(), Some(Component::Normal(value)) if value == name);
        if !is_normal || components.next().is_some() || contains_nul(name) {
            return Err(StorageError::invalid_name(name));
        }
        Ok(Self(name.to_os_string()))
    }

    /// Get the operating-system string for this component.
    #[must_use]
    pub fn as_os_str(&self) -> &OsStr {
        &self.0
    }
}

impl TryFrom<&str> for ObjectName {
    type Error = StorageError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<String> for ObjectName {
    type Error = StorageError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

#[cfg(unix)]
fn contains_nul(value: &OsStr) -> bool {
    use std::os::unix::ffi::OsStrExt;
    value.as_bytes().contains(&0)
}

#[cfg(not(unix))]
fn contains_nul(value: &OsStr) -> bool {
    value.to_string_lossy().contains('\0')
}

/// Exact bytes and their calculated digest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactObject {
    digest: ContentDigest,
    bytes: Vec<u8>,
}

impl ExactObject {
    /// Build an object from its complete bytes.
    #[must_use]
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Self {
        let bytes = bytes.into();
        let digest = digest_bytes(&bytes);
        Self { digest, bytes }
    }

    /// Get the content digest.
    #[must_use]
    pub const fn digest(&self) -> ContentDigest {
        self.digest
    }

    /// Get the complete bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// A verified private temporary object owned by one anchored storage directory.
#[derive(Debug)]
#[cfg_attr(
    not(any(target_vendor = "apple", target_os = "linux", target_os = "android")),
    allow(dead_code)
)]
pub struct OwnedTemporary {
    pub(crate) name: ObjectName,
    pub(crate) identity: ObjectIdentity,
    pub(crate) object: ExactObject,
    pub(crate) owner_root: RootIdentity,
    pub(crate) owner_directory: ObjectIdentity,
}

impl OwnedTemporary {
    /// Get the owned temporary name.
    #[must_use]
    pub const fn name(&self) -> &ObjectName {
        &self.name
    }

    /// Get the exact temporary identity.
    #[must_use]
    pub const fn identity(&self) -> ObjectIdentity {
        self.identity
    }

    /// Get the complete temporary bytes and digest.
    #[must_use]
    pub const fn object(&self) -> &ExactObject {
        &self.object
    }
}

/// An exact private tree captured in one anchored storage directory.
#[derive(Debug)]
#[cfg_attr(
    not(any(target_vendor = "apple", target_os = "linux", target_os = "android")),
    allow(dead_code)
)]
pub struct PrivateTreeManifest {
    pub(crate) owner_root: RootIdentity,
    pub(crate) owner_directory: ObjectIdentity,
    pub(crate) root: PrivateTreeNode,
    pub(crate) total_file_bytes: usize,
}

impl PrivateTreeManifest {
    /// Get the top-level tree name.
    #[must_use]
    pub const fn name(&self) -> &ObjectName {
        &self.root.name
    }

    /// Get the exact top-level tree identity.
    #[must_use]
    pub const fn identity(&self) -> ObjectIdentity {
        self.root.identity
    }

    /// Get the number of objects in the exact tree.
    #[must_use]
    pub fn object_count(&self) -> usize {
        self.root.object_count()
    }

    /// Get the total regular-file byte count in the exact tree.
    #[must_use]
    pub const fn total_file_bytes(&self) -> usize {
        self.total_file_bytes
    }
}

/// Bounds for one private tree manifest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PrivateTreeLimits {
    /// Maximum number of files and directories.
    pub maximum_objects: usize,
    /// Maximum bytes in one regular file.
    pub maximum_file_bytes: usize,
    /// Maximum total bytes in all regular files.
    pub maximum_total_bytes: usize,
}

impl PrivateTreeLimits {
    /// Make exact manifest bounds.
    #[must_use]
    pub const fn new(
        maximum_objects: usize,
        maximum_file_bytes: usize,
        maximum_total_bytes: usize,
    ) -> Self {
        Self {
            maximum_objects,
            maximum_file_bytes,
            maximum_total_bytes,
        }
    }
}

#[derive(Debug)]
#[cfg_attr(
    not(any(target_vendor = "apple", target_os = "linux", target_os = "android")),
    allow(dead_code)
)]
pub(crate) struct PrivateTreeNode {
    pub(crate) name: ObjectName,
    pub(crate) identity: ObjectIdentity,
    pub(crate) kind: ObjectKind,
    pub(crate) file: Option<PrivateFileManifest>,
    pub(crate) children: Vec<PrivateTreeNode>,
}

#[derive(Debug)]
#[cfg_attr(
    not(any(target_vendor = "apple", target_os = "linux", target_os = "android")),
    allow(dead_code)
)]
pub(crate) struct PrivateFileManifest {
    pub(crate) digest: ContentDigest,
    pub(crate) size: usize,
}

impl PrivateTreeNode {
    fn object_count(&self) -> usize {
        self.children.iter().fold(1_usize, |count, child| {
            count.saturating_add(child.object_count())
        })
    }
}

/// The device and inode of an open storage root.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RootIdentity {
    /// Device number.
    pub device: u64,
    /// Inode number.
    pub inode: u64,
}

/// The device and inode of a managed object.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ObjectIdentity {
    /// Device number.
    pub device: u64,
    /// Inode number.
    pub inode: u64,
}

/// A managed object type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObjectKind {
    /// A regular private file.
    RegularFile,
    /// A private directory.
    Directory,
}

/// Verified metadata for a managed object.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObjectInspection {
    /// Stable identity for the inspected name binding.
    pub identity: ObjectIdentity,
    /// Verified object type.
    pub kind: ObjectKind,
    /// Exact permission bits.
    pub mode: u32,
    /// Link count at inspection time.
    pub link_count: u64,
    /// File size in bytes.
    pub size: u64,
}

/// The expected value for a mutable compare-and-swap object.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExpectedValue {
    /// The object name must be absent.
    Absent,
    /// The object must contain these exact bytes.
    Exact(ExactObject),
}

/// The result of a compare-and-swap operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CasOutcome {
    /// The store replaced the expected value.
    Exchanged,
    /// Exact readback found the requested new value.
    AlreadyExact,
}

/// The result of an immutable no-replace publication.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PutOutcome {
    /// The store published the object.
    Published,
    /// The exact object was already durable.
    AlreadyExact,
}

/// A storage operation used by errors and fault rules.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum StorageOperation {
    /// Open or create the storage root.
    OpenRoot,
    /// Validate the anchored session.
    ValidateRoot,
    /// Create or open a child directory.
    CreateDirectory,
    /// Read a directory.
    ListDirectory,
    /// Inspect a managed object.
    InspectObject,
    /// Read a managed object.
    ReadObject,
    /// Publish an immutable object.
    PublishImmutable,
    /// Acquire the anchored writer lease.
    AcquireWriter,
    /// Validate the anchored writer lease.
    ValidateWriter,
    /// Compare and exchange a mutable file.
    CompareExchange,
    /// Remove an owned temporary file.
    RemoveTemporary,
    /// Remove an exact private tree.
    RemoveTree,
}

/// A durability boundary used by errors and fault rules.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DurabilityStep {
    /// No durable mutation has started.
    Selection,
    /// Validate identity before a mutation.
    BeforeMutation,
    /// Create a private directory or file.
    Creation,
    /// Synchronize complete file data.
    ObjectData,
    /// Verify complete bytes after synchronization.
    ObjectReadback,
    /// Publish an immutable name.
    ObjectPublication,
    /// Synchronize the object directory.
    ParentDirectory,
    /// Rename the mutable authorization object.
    AuthorizationRename,
    /// Read the authorization object after an error.
    AuthorizationReadback,
    /// Apply the second durability barrier after readback.
    RecoveryBarrier,
    /// Unlink a verified object.
    Deletion,
    /// Validate identity after a mutation.
    AfterMutation,
}

/// Context available at one storage boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageContext {
    /// Requested root path when the caller supplied it.
    pub requested_root: Option<PathBuf>,
    /// Bound root identity when root open succeeded.
    pub root: Option<RootIdentity>,
    /// Selected object digest when bytes are known.
    pub object: Option<ContentDigest>,
    /// Selected path component when one is known.
    pub component: Option<ObjectName>,
    /// Active storage operation.
    pub operation: StorageOperation,
    /// Active durability step.
    pub step: DurabilityStep,
}

impl StorageContext {
    /// Make context before a root identity or component is available.
    #[must_use]
    pub const fn root_open() -> Self {
        Self {
            requested_root: None,
            root: None,
            object: None,
            component: None,
            operation: StorageOperation::OpenRoot,
            step: DurabilityStep::Selection,
        }
    }

    pub(crate) fn root_open_at(path: &Path) -> Self {
        Self {
            requested_root: Some(path.to_path_buf()),
            ..Self::root_open()
        }
    }

    #[cfg(any(target_vendor = "apple", target_os = "linux", target_os = "android"))]
    pub(crate) fn new(
        root: RootIdentity,
        component: Option<&ObjectName>,
        object: Option<ContentDigest>,
        operation: StorageOperation,
        step: DurabilityStep,
    ) -> Self {
        Self {
            requested_root: None,
            root: Some(root),
            object,
            component: component.cloned(),
            operation,
            step,
        }
    }
}
