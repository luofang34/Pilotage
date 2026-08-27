//! What a Linux process says its command line is.
//!
//! Reading `/proc/<pid>/cmdline` and saying what was read are one concern:
//! both have to agree about where an argument ends, and a describer that
//! split differently from the digest would name something other than what was
//! compared.

/// The NUL-separated arguments in a `/proc/<pid>/cmdline` image.
///
/// The kernel terminates the last argument as well as separating them, so a
/// trailing empty argument is punctuation rather than an argument nobody
/// passed.
pub(super) fn split(command: &[u8]) -> Vec<&[u8]> {
    let mut arguments = command.split(|byte| *byte == 0).collect::<Vec<_>>();
    if arguments.last().is_some_and(|argument| argument.is_empty()) {
        arguments.pop();
    }
    arguments
}

/// The observed arguments, readable, and bounded.
///
/// Lossy because a command line is bytes, not text, and a diagnostic that
/// refuses to print anything for an argument it cannot decode is a diagnostic
/// that goes quiet exactly when something unusual happened. Bounded because
/// this reaches a log, and an process with a pathological command line should
/// not be able to flood it.
pub(super) fn describe(arguments: &[&[u8]]) -> String {
    const MAX_ARGUMENTS: usize = 8;
    const MAX_ARGUMENT_BYTES: usize = 96;
    let mut described = arguments
        .iter()
        .take(MAX_ARGUMENTS)
        .map(|argument| {
            let clipped = &argument[..argument.len().min(MAX_ARGUMENT_BYTES)];
            let text = String::from_utf8_lossy(clipped);
            if argument.len() > MAX_ARGUMENT_BYTES {
                format!("{text}...")
            } else {
                text.into_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    if arguments.len() > MAX_ARGUMENTS {
        described.push_str(" ...");
    }
    if described.is_empty() {
        "no arguments".to_owned()
    } else {
        described
    }
}

#[cfg(test)]
mod tests {
    use super::{describe, split};

    #[test]
    fn the_kernels_trailing_terminator_is_not_an_argument() {
        // `/proc/<pid>/cmdline` terminates the last argument as well as
        // separating them, so a naive split reports one more argument than
        // was passed — and an empty one at that, which would then be hashed.
        assert_eq!(
            split(b"/bin/sleep\x0060\x00"),
            vec![&b"/bin/sleep"[..], b"60"]
        );
        // An argument that really is empty, in the middle, is kept.
        assert_eq!(split(b"a\x00\x00b\x00"), vec![&b"a"[..], b"", b"b"]);
        assert_eq!(split(b""), Vec::<&[u8]>::new());
    }

    #[test]
    fn a_command_line_is_described_however_it_is_encoded() {
        // A command line is bytes. A describer that printed nothing for what
        // it could not decode would go quiet exactly when something unusual
        // had happened, which is when it is being read.
        assert_eq!(describe(&[b"/bin/sleep", b"60"]), "/bin/sleep 60");
        assert!(describe(&[b"\xff\xfe"]).contains('\u{fffd}'));
        assert_eq!(describe(&[]), "no arguments");
    }

    #[test]
    fn a_pathological_command_line_cannot_flood_the_log() {
        // This reaches a log, and the process being described is by
        // definition not the one that was expected.
        let long = vec![b'x'; 4096];
        let described = describe(&[&long]);
        assert!(described.len() < 200, "one argument was not clipped");
        assert!(described.ends_with("..."));

        let many: Vec<&[u8]> = (0..64).map(|_| &b"arg"[..]).collect();
        let described = describe(&many);
        assert!(described.len() < 200, "the argument list was not clipped");
        assert!(described.ends_with(" ..."));
    }
}
