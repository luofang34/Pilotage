use std::ffi::OsStr;
use std::path::Path;

use pilotage_durable_storage::{
    DurableDirectory, DurableStore, ExactObject, ObjectInspection, ObjectKind, ObjectName,
    WriterLease,
};

use super::{
    HeadPointer, MAX_DOCUMENT_BYTES, decode, exact_head, invalid_journal, object_name,
    storage_error, writer_error,
};
use crate::TuneError;

pub(super) const LAYOUT_MARKER: &str = ".flight-tune-journal-v2";
const WRITER_LOCK: &str = ".pilotage-writer-lock";
const HEAD: &str = "HEAD.json";
const DIRECTORIES: [&str; 3] = ["candidates", "stages", "entries"];

pub(super) struct OpenedLayout {
    pub(super) root: DurableDirectory,
    pub(super) marker: DurableDirectory,
    pub(super) candidates: DurableDirectory,
    pub(super) stages: DurableDirectory,
    pub(super) entries: DurableDirectory,
    pub(super) writer: WriterLease,
}

pub(super) fn open(root_path: &Path, store: &DurableStore) -> Result<OpenedLayout, TuneError> {
    open_with_hook(root_path, store, || {})
}

#[cfg(test)]
pub(super) fn open_with_acquisition_hook_for_test(
    root_path: &Path,
    store: &DurableStore,
    after_acquisition: impl FnOnce(),
) -> Result<OpenedLayout, TuneError> {
    open_with_hook(root_path, store, after_acquisition)
}

fn open_with_hook(
    root_path: &Path,
    store: &DurableStore,
    after_acquisition: impl FnOnce(),
) -> Result<OpenedLayout, TuneError> {
    let root = store.root_directory();
    let before = inspect_layout(&root)?;
    let writer = store
        .acquire_writer()
        .map_err(|source| writer_error(root_path, source))?;
    after_acquisition();
    let after = inspect_layout(&root)?;
    validate_acquisition(&before, &after)?;
    let marker = root
        .child(&writer, &object_name(LAYOUT_MARKER)?)
        .map_err(storage_error)?;
    require_empty_marker(&marker)?;
    let candidates = open_directory(&root, &writer, DIRECTORIES[0])?;
    let stages = open_directory(&root, &writer, DIRECTORIES[1])?;
    let entries = open_directory(&root, &writer, DIRECTORIES[2])?;
    Ok(OpenedLayout {
        root,
        marker,
        candidates,
        stages,
        entries,
        writer,
    })
}

pub(super) fn verify_handles(
    marker: &DurableDirectory,
    candidates: &DurableDirectory,
    stages: &DurableDirectory,
    entries: &DurableDirectory,
) -> Result<(), TuneError> {
    require_empty_marker(marker)?;
    candidates.list().map_err(storage_error)?;
    stages.list().map_err(storage_error)?;
    entries.list().map_err(storage_error)?;
    Ok(())
}

pub(super) fn verify_authorized(root: &DurableDirectory) -> Result<(), TuneError> {
    let layout = inspect_layout(root)?;
    let has_head = layout
        .objects
        .iter()
        .any(|object| is_name(&object.name, HEAD));
    if layout.phase == LayoutPhase::Marked && has_head {
        Ok(())
    } else {
        Err(invalid_layout(
            "the live journal does not have an authorized marked layout",
        ))
    }
}

pub(super) fn verify_initial_authorization(root: &DurableDirectory) -> Result<(), TuneError> {
    let layout = inspect_layout(root)?;
    let has_head = layout
        .objects
        .iter()
        .any(|object| is_name(&object.name, HEAD));
    let has_all_directories = DIRECTORIES.iter().all(|name| {
        layout
            .objects
            .iter()
            .any(|object| is_name(&object.name, name))
    });
    if layout.phase == LayoutPhase::Marked && !has_head && has_all_directories {
        Ok(())
    } else {
        Err(invalid_layout(
            "the initial journal does not have a complete marked layout",
        ))
    }
}

fn open_directory(
    root: &DurableDirectory,
    writer: &WriterLease,
    name: &str,
) -> Result<DurableDirectory, TuneError> {
    root.child(writer, &object_name(name)?)
        .map_err(storage_error)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LayoutSnapshot {
    phase: LayoutPhase,
    objects: Vec<LayoutObject>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LayoutPhase {
    Bootstrap,
    Marked,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LayoutObject {
    name: ObjectName,
    inspection: ObjectInspection,
    exact: Option<ExactObject>,
}

fn inspect_layout(root: &DurableDirectory) -> Result<LayoutSnapshot, TuneError> {
    let names = root.list().map_err(storage_error)?;
    let marked = contains(&names, LAYOUT_MARKER);
    if marked {
        inspect_marked(root, names)
    } else {
        inspect_bootstrap(root, names)
    }
}

fn inspect_bootstrap(
    root: &DurableDirectory,
    names: Vec<ObjectName>,
) -> Result<LayoutSnapshot, TuneError> {
    if names.is_empty() {
        return Ok(LayoutSnapshot {
            phase: LayoutPhase::Bootstrap,
            objects: Vec::new(),
        });
    }
    if names.len() != 1 || !is_name(&names[0], WRITER_LOCK) {
        return Err(invalid_layout("the root has evidence from another layout"));
    }
    let lock = inspect_file(root, names[0].clone(), Some(0))?;
    Ok(LayoutSnapshot {
        phase: LayoutPhase::Bootstrap,
        objects: vec![lock],
    })
}

fn inspect_marked(
    root: &DurableDirectory,
    names: Vec<ObjectName>,
) -> Result<LayoutSnapshot, TuneError> {
    let has_head = contains(&names, HEAD);
    if !contains(&names, WRITER_LOCK) {
        return Err(invalid_layout("a marked layout has no writer lock"));
    }
    if has_head && DIRECTORIES.iter().any(|name| !contains(&names, name)) {
        return Err(invalid_layout(
            "an authorized layout does not have all data directories",
        ));
    }
    if !has_head && !has_bootstrap_prefix(&names) {
        return Err(invalid_layout(
            "a partial layout does not have an ordered directory prefix",
        ));
    }
    let mut objects = Vec::with_capacity(names.len());
    for name in names {
        let object = inspect_marked_object(root, name)?;
        objects.push(object);
    }
    Ok(LayoutSnapshot {
        phase: LayoutPhase::Marked,
        objects,
    })
}

fn inspect_marked_object(
    root: &DurableDirectory,
    name: ObjectName,
) -> Result<LayoutObject, TuneError> {
    if is_name(&name, LAYOUT_MARKER) || DIRECTORIES.iter().any(|value| is_name(&name, value)) {
        return inspect_directory(root, name);
    }
    if is_name(&name, WRITER_LOCK) {
        return inspect_file(root, name, Some(0));
    }
    if is_name(&name, HEAD) {
        return inspect_head(root, name);
    }
    if is_strict_temporary(&name) {
        return inspect_file(root, name, None);
    }
    Err(invalid_layout(
        "a marked layout has an unsupported root object",
    ))
}

fn inspect_directory(root: &DurableDirectory, name: ObjectName) -> Result<LayoutObject, TuneError> {
    let inspection = root.inspect(&name).map_err(storage_error)?;
    require_kind(inspection.kind, ObjectKind::Directory)?;
    Ok(LayoutObject {
        name,
        inspection,
        exact: None,
    })
}

fn require_empty_marker(marker: &DurableDirectory) -> Result<(), TuneError> {
    if marker.list().map_err(storage_error)?.is_empty() {
        Ok(())
    } else {
        Err(invalid_layout("the layout marker is not empty"))
    }
}

fn inspect_file(
    root: &DurableDirectory,
    name: ObjectName,
    maximum_bytes: Option<usize>,
) -> Result<LayoutObject, TuneError> {
    let inspection = root.inspect(&name).map_err(storage_error)?;
    require_kind(inspection.kind, ObjectKind::RegularFile)?;
    let exact = maximum_bytes
        .map(|limit| root.read_exact(&name, limit).map_err(storage_error))
        .transpose()?;
    Ok(LayoutObject {
        name,
        inspection,
        exact,
    })
}

fn inspect_head(root: &DurableDirectory, name: ObjectName) -> Result<LayoutObject, TuneError> {
    let mut object = inspect_file(root, name.clone(), Some(MAX_DOCUMENT_BYTES))?;
    let exact = object
        .exact
        .as_ref()
        .ok_or_else(|| invalid_layout("the journal head was not read"))?;
    let head: HeadPointer = decode("journal head", &name, exact.bytes())?;
    if exact != &exact_head(head.digest)? {
        return Err(invalid_journal(
            "the journal head does not use canonical bytes",
        ));
    }
    object.exact = Some(exact.clone());
    Ok(object)
}

fn validate_acquisition(before: &LayoutSnapshot, after: &LayoutSnapshot) -> Result<(), TuneError> {
    match before.phase {
        LayoutPhase::Bootstrap if before.objects.is_empty() && exact_lock_only(after) => Ok(()),
        LayoutPhase::Bootstrap | LayoutPhase::Marked if before == after => Ok(()),
        LayoutPhase::Bootstrap | LayoutPhase::Marked => Err(invalid_layout(
            "the journal layout changed during writer acquisition",
        )),
    }
}

fn exact_lock_only(layout: &LayoutSnapshot) -> bool {
    layout.phase == LayoutPhase::Bootstrap
        && layout.objects.len() == 1
        && is_name(&layout.objects[0].name, WRITER_LOCK)
}

fn require_kind(actual: ObjectKind, expected: ObjectKind) -> Result<(), TuneError> {
    if actual == expected {
        Ok(())
    } else {
        Err(invalid_layout("a journal layout object has the wrong type"))
    }
}

fn contains(names: &[ObjectName], expected: &str) -> bool {
    names.iter().any(|name| is_name(name, expected))
}

fn has_bootstrap_prefix(names: &[ObjectName]) -> bool {
    let present = DIRECTORIES.map(|name| contains(names, name));
    !present.windows(2).any(|pair| !pair[0] && pair[1])
}

fn is_name(name: &ObjectName, expected: &str) -> bool {
    name.as_os_str() == OsStr::new(expected)
}

fn is_strict_temporary(name: &ObjectName) -> bool {
    let Some(value) = name.as_os_str().to_str() else {
        return false;
    };
    let Some(value) = value.strip_prefix(".pilotage-tmp-") else {
        return false;
    };
    let Some((process, counter)) = value.split_once('-') else {
        return false;
    };
    !process.is_empty()
        && process.bytes().all(|byte| byte.is_ascii_digit())
        && counter.len() == 16
        && counter
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn invalid_layout(detail: &'static str) -> TuneError {
    invalid_journal(format!("journal layout is not valid: {detail}"))
}
