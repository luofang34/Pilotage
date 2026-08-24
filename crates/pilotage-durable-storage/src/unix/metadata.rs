use rustix::fs::{FileType, Mode, Stat};

use crate::{
    ObjectIdentity, ObjectInspection, ObjectKind, StorageContext, StorageError, StorageResult,
};

pub(crate) const PRIVATE_DIRECTORY_MODE: u32 = 0o700;
pub(crate) const PRIVATE_FILE_MODE: u32 = 0o600;

pub(crate) fn directory_mode() -> Mode {
    Mode::RWXU
}

pub(crate) fn file_mode() -> Mode {
    Mode::RUSR | Mode::WUSR
}

#[allow(clippy::unnecessary_cast)]
pub(crate) fn identity(stat: &Stat) -> ObjectIdentity {
    ObjectIdentity {
        device: stat.st_dev as u64,
        inode: stat.st_ino as u64,
    }
}

pub(crate) fn mode(stat: &Stat) -> u32 {
    (stat.st_mode as u32) & 0o7777
}

pub(crate) fn inspect_private(
    stat: &Stat,
    context: &StorageContext,
) -> StorageResult<ObjectInspection> {
    let file_type = FileType::from_raw_mode(stat.st_mode);
    let (kind, required) = if file_type.is_file() {
        (ObjectKind::RegularFile, PRIVATE_FILE_MODE)
    } else if file_type.is_dir() {
        (ObjectKind::Directory, PRIVATE_DIRECTORY_MODE)
    } else {
        return Err(StorageError::WrongType {
            context: context.clone(),
        });
    };
    let actual = mode(stat);
    if actual != required {
        return Err(StorageError::WrongMode {
            required,
            actual,
            context: context.clone(),
        });
    }
    let link_count = stat.st_nlink as u64;
    if kind == ObjectKind::RegularFile && link_count != 1 {
        return Err(StorageError::LinkedObject {
            actual: link_count,
            context: context.clone(),
        });
    }
    Ok(ObjectInspection {
        identity: identity(stat),
        kind,
        mode: actual,
        link_count,
        size: stat.st_size.max(0) as u64,
    })
}

pub(crate) fn inspect_temporary(
    stat: &Stat,
    context: &StorageContext,
) -> StorageResult<ObjectInspection> {
    if !FileType::from_raw_mode(stat.st_mode).is_file() {
        return Err(StorageError::WrongType {
            context: context.clone(),
        });
    }
    let actual = mode(stat);
    if actual != PRIVATE_FILE_MODE {
        return Err(StorageError::WrongMode {
            required: PRIVATE_FILE_MODE,
            actual,
            context: context.clone(),
        });
    }
    Ok(ObjectInspection {
        identity: identity(stat),
        kind: ObjectKind::RegularFile,
        mode: actual,
        link_count: stat.st_nlink as u64,
        size: stat.st_size.max(0) as u64,
    })
}
