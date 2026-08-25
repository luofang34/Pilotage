use std::path::{Path, PathBuf};

/// One terminal authority boundary after which the test rig changes the journal head.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalExternalAction {
    /// Read the capabilities that define the terminal plan.
    PlanRead,
    /// Bind the terminal plan.
    Bind,
    /// Stop the simulator.
    SimulatorStop,
    /// Stop the control path.
    ControlStop,
    /// Stop trace collection.
    TraceStop,
    /// Read child health.
    ChildHealth,
    /// Stop the trace path.
    TraceShutdown,
    /// Terminate the child group.
    ChildTerminate,
    /// Read the causal evidence digest.
    CausalRead,
    /// Recover terminal receipts.
    ReceiptRecover,
    /// Seal a terminal receipt.
    ReceiptSeal,
}

#[derive(Debug, Default)]
pub(super) struct FakeTerminalHeadPoison {
    pending: Option<(TerminalExternalAction, PathBuf)>,
}

impl FakeTerminalHeadPoison {
    pub(super) fn arm(&mut self, action: TerminalExternalAction, root: &Path) {
        self.pending = Some((action, root.to_path_buf()));
    }

    pub(super) fn take(&mut self, action: TerminalExternalAction) -> Option<PathBuf> {
        if self
            .pending
            .as_ref()
            .is_some_and(|(expected, _)| *expected == action)
        {
            return self.pending.take().map(|(_, root)| root);
        }
        None
    }
}

pub(super) fn poison_terminal_head(root: Option<PathBuf>) {
    let Some(root) = root else {
        return;
    };
    let head = root.join("HEAD.json");
    let mut bytes = std::fs::read(&head).expect("read journal head");
    let digest_tail = bytes.len().checked_sub(3).expect("HEAD digest byte");
    bytes[digest_tail] = if bytes[digest_tail] == b'0' {
        b'1'
    } else {
        b'0'
    };
    std::fs::write(head, bytes).expect("change journal head");
}
