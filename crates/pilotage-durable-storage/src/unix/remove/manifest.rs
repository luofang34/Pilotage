use rustix::fs::{AtFlags, statat};

use crate::{
    DurabilityStep, ObjectInspection, ObjectKind, ObjectName, PrivateTreeLimits,
    PrivateTreeManifest, StorageContext, StorageError, StorageOperation, StorageResult,
    types::{PrivateFileManifest, PrivateTreeNode},
};

use super::super::anchor::open_directory;
use super::super::directory::DurableDirectory;
use super::super::metadata::inspect_private;
use super::super::objects;

struct TreeBudget {
    remaining_objects: usize,
    remaining_bytes: usize,
}

pub(crate) fn inspect_private_tree(
    directory: &DurableDirectory,
    name: &ObjectName,
    limits: PrivateTreeLimits,
) -> StorageResult<PrivateTreeManifest> {
    let context = directory.handle.context(
        Some(name),
        StorageOperation::InspectObject,
        DurabilityStep::Selection,
    );
    let mut budget = TreeBudget {
        remaining_objects: limits.maximum_objects,
        remaining_bytes: limits.maximum_total_bytes,
    };
    let root = build(directory, name, limits, &mut budget, &context)?;
    verify(directory, &root, &context)?;
    Ok(PrivateTreeManifest {
        owner_root: directory.handle.anchor.identity,
        owner_directory: directory.handle.identity,
        root,
        total_file_bytes: limits
            .maximum_total_bytes
            .saturating_sub(budget.remaining_bytes),
    })
}

pub(super) fn validate_owner(
    directory: &DurableDirectory,
    manifest: &PrivateTreeManifest,
    context: &StorageContext,
) -> StorageResult<()> {
    if manifest.owner_root != directory.handle.anchor.identity {
        return Err(StorageError::Corruption {
            reason: "private tree manifest belongs to a different storage root",
            context: context.clone(),
        });
    }
    if manifest.owner_directory != directory.handle.identity {
        return Err(StorageError::IdentityChanged {
            expected: manifest.owner_directory,
            actual: Some(directory.handle.identity),
            context: context.clone(),
        });
    }
    Ok(())
}

fn build(
    directory: &DurableDirectory,
    name: &ObjectName,
    limits: PrivateTreeLimits,
    budget: &mut TreeBudget,
    context: &StorageContext,
) -> StorageResult<PrivateTreeNode> {
    budget.take_object(context)?;
    directory.handle.validate(context)?;
    let stat = statat(
        &directory.handle.fd,
        name.as_os_str(),
        AtFlags::SYMLINK_NOFOLLOW,
    )
    .map_err(|source| StorageError::Io {
        context: context.clone(),
        source: source.into(),
    })?;
    let inspected = inspect_private(&stat, context)?;
    let mut node = PrivateTreeNode {
        name: name.clone(),
        identity: inspected.identity,
        kind: inspected.kind,
        file: None,
        children: Vec::new(),
    };
    if inspected.kind == ObjectKind::RegularFile {
        node.file = Some(capture_file(
            directory, name, inspected, limits, budget, context,
        )?);
    } else {
        build_children(directory, limits, budget, context, &mut node)?;
    }
    directory.handle.validate(context)?;
    Ok(node)
}

fn build_children(
    directory: &DurableDirectory,
    limits: PrivateTreeLimits,
    budget: &mut TreeBudget,
    context: &StorageContext,
    node: &mut PrivateTreeNode,
) -> StorageResult<()> {
    let child = open_manifest_directory(directory, node, context)?;
    for child_name in child.list()? {
        let child_context = child.handle.context(
            Some(&child_name),
            StorageOperation::RemoveTree,
            DurabilityStep::BeforeMutation,
        );
        node.children
            .push(build(&child, &child_name, limits, budget, &child_context)?);
    }
    Ok(())
}

impl TreeBudget {
    fn take_object(&mut self, context: &StorageContext) -> StorageResult<()> {
        self.remaining_objects =
            self.remaining_objects
                .checked_sub(1)
                .ok_or_else(|| StorageError::Corruption {
                    reason: "private tree exceeds the manifest object limit",
                    context: context.clone(),
                })?;
        Ok(())
    }
}

fn capture_file(
    directory: &DurableDirectory,
    name: &ObjectName,
    inspected: ObjectInspection,
    limits: PrivateTreeLimits,
    budget: &mut TreeBudget,
    context: &StorageContext,
) -> StorageResult<PrivateFileManifest> {
    let size = usize::try_from(inspected.size).map_err(|_| StorageError::ObjectTooLarge {
        limit: limits.maximum_file_bytes,
        actual: inspected.size,
        context: context.clone(),
    })?;
    let limit = limits.maximum_file_bytes.min(budget.remaining_bytes);
    if size > limit {
        return Err(StorageError::ObjectTooLarge {
            limit,
            actual: inspected.size,
            context: context.clone(),
        });
    }
    let object = objects::read_exact(&directory.handle, name, size)?;
    budget.remaining_bytes -= size;
    let digest = object.digest();
    let after = objects::inspect(&directory.handle, name).map_err(|error| {
        error.at_object(
            StorageOperation::RemoveTree,
            digest,
            DurabilityStep::ObjectReadback,
        )
    })?;
    verify_captured_file(directory, name, inspected, after, digest)?;
    Ok(PrivateFileManifest { digest, size })
}

fn verify_captured_file(
    directory: &DurableDirectory,
    name: &ObjectName,
    before: ObjectInspection,
    after: ObjectInspection,
    digest: crate::ContentDigest,
) -> StorageResult<()> {
    let context = directory.handle.context_with_object(
        Some(name),
        Some(digest),
        StorageOperation::RemoveTree,
        DurabilityStep::ObjectReadback,
    );
    if after.identity != before.identity {
        return Err(StorageError::IdentityChanged {
            expected: before.identity,
            actual: Some(after.identity),
            context,
        });
    }
    if after.size != before.size {
        return Err(StorageError::ContentMismatch { context });
    }
    Ok(())
}

pub(super) fn verify(
    directory: &DurableDirectory,
    node: &PrivateTreeNode,
    context: &StorageContext,
) -> StorageResult<()> {
    directory.handle.validate(context)?;
    verify_node(directory, node, context)?;
    if node.kind == ObjectKind::RegularFile {
        verify_regular_manifest(directory, node, context)?;
        return directory.handle.validate(context);
    }
    if node.file.is_some() {
        return Err(StorageError::Corruption {
            reason: "a directory manifest has a regular-file value",
            context: context.clone(),
        });
    }
    let child = open_manifest_directory(directory, node, context)?;
    verify_child_names(&child, node, context)?;
    for descendant in &node.children {
        let child_context = child.handle.context(
            Some(&descendant.name),
            StorageOperation::RemoveTree,
            DurabilityStep::BeforeMutation,
        );
        verify(&child, descendant, &child_context)?;
    }
    directory.handle.validate(context)
}

fn verify_regular_manifest(
    directory: &DurableDirectory,
    node: &PrivateTreeNode,
    context: &StorageContext,
) -> StorageResult<()> {
    let Some(file) = node.file.as_ref() else {
        return Err(StorageError::Corruption {
            reason: "a regular-file manifest has no exact file value",
            context: context.clone(),
        });
    };
    if !node.children.is_empty() {
        return Err(StorageError::Corruption {
            reason: "a regular-file manifest has descendants",
            context: context.clone(),
        });
    }
    verify_file(directory, node, file, context)
}

fn verify_child_names(
    child: &DurableDirectory,
    node: &PrivateTreeNode,
    context: &StorageContext,
) -> StorageResult<()> {
    let actual = child.list()?;
    let exact = actual.len() == node.children.len()
        && actual
            .iter()
            .zip(&node.children)
            .all(|(name, expected)| name == &expected.name);
    if exact {
        Ok(())
    } else {
        Err(StorageError::Corruption {
            reason: "private tree differs from its exact manifest",
            context: context.clone(),
        })
    }
}

fn verify_file(
    directory: &DurableDirectory,
    node: &PrivateTreeNode,
    file: &PrivateFileManifest,
    context: &StorageContext,
) -> StorageResult<()> {
    let object_context = directory.handle.context_with_object(
        Some(&node.name),
        Some(file.digest),
        StorageOperation::RemoveTree,
        DurabilityStep::ObjectReadback,
    );
    let actual = objects::read_digest(&directory.handle, &node.name, file.digest, file.size)
        .map_err(|error| {
            error.at_object(
                StorageOperation::RemoveTree,
                file.digest,
                DurabilityStep::ObjectReadback,
            )
        })?;
    if actual.bytes().len() != file.size {
        return Err(StorageError::ContentMismatch {
            context: object_context,
        });
    }
    verify_node(directory, node, &object_context)?;
    directory.handle.validate(context)
}

pub(super) fn verify_exact_node(
    directory: &DurableDirectory,
    node: &PrivateTreeNode,
    context: &StorageContext,
) -> StorageResult<()> {
    verify_node(directory, node, context)?;
    if let Some(file) = &node.file {
        verify_file(directory, node, file, context)?;
    }
    Ok(())
}

fn verify_node(
    directory: &DurableDirectory,
    node: &PrivateTreeNode,
    context: &StorageContext,
) -> StorageResult<()> {
    let stat = statat(
        &directory.handle.fd,
        node.name.as_os_str(),
        AtFlags::SYMLINK_NOFOLLOW,
    )
    .map_err(|source| StorageError::Io {
        context: context.clone(),
        source: source.into(),
    })?;
    let inspected = inspect_private(&stat, context)?;
    if inspected.identity != node.identity || inspected.kind != node.kind {
        return Err(StorageError::IdentityChanged {
            expected: node.identity,
            actual: Some(inspected.identity),
            context: context.clone(),
        });
    }
    Ok(())
}

pub(super) fn open_manifest_directory(
    parent: &DurableDirectory,
    node: &PrivateTreeNode,
    context: &StorageContext,
) -> StorageResult<DurableDirectory> {
    let fd = open_directory(&parent.handle.fd, &node.name).map_err(|source| StorageError::Io {
        context: context.clone(),
        source: source.into(),
    })?;
    Ok(DurableDirectory::new(parent.handle.child(
        fd,
        node.name.clone(),
        node.identity,
    )))
}
