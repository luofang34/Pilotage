use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::{DurabilityStep, StorageContext, StorageError, StorageOperation, StorageResult};

/// The effect of one injected storage fault.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FaultAction {
    /// Return an error before the file-system call starts.
    FailBefore,
    /// Complete the file-system call and lose its acknowledgement.
    LoseAckAfter,
}

/// One typed fault at a numbered real boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FaultRule {
    operation: StorageOperation,
    step: DurabilityStep,
    occurrence: u64,
    action: FaultAction,
}

#[cfg_attr(not(test), allow(dead_code))]
impl FaultRule {
    /// Inject a fault at the first matching boundary.
    #[must_use]
    pub const fn once(
        operation: StorageOperation,
        step: DurabilityStep,
        action: FaultAction,
    ) -> Self {
        Self::on_occurrence(operation, step, 1, action)
    }

    /// Inject a fault at a selected one-based matching boundary.
    #[must_use]
    pub const fn on_occurrence(
        operation: StorageOperation,
        step: DurabilityStep,
        occurrence: u64,
        action: FaultAction,
    ) -> Self {
        Self {
            operation,
            step,
            occurrence,
            action,
        }
    }
}

#[derive(Debug)]
#[cfg_attr(
    not(any(target_vendor = "apple", target_os = "linux", target_os = "android")),
    allow(dead_code)
)]
struct RuleState {
    rule: FaultRule,
    consumed: bool,
}

#[cfg(test)]
#[derive(Default)]
struct TestHook(Option<Box<dyn FnOnce() + Send>>);

#[cfg(test)]
impl std::fmt::Debug for TestHook {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TestHook")
            .field("armed", &self.0.is_some())
            .finish()
    }
}

#[derive(Debug, Default)]
#[cfg_attr(
    not(any(target_vendor = "apple", target_os = "linux", target_os = "android")),
    allow(dead_code)
)]
struct FaultState {
    observed: BTreeMap<(StorageOperation, DurabilityStep), u64>,
    rules: Vec<RuleState>,
    #[cfg(test)]
    test_hook: TestHook,
}

/// A shared deterministic controller for real storage boundaries.
#[derive(Clone, Debug, Default)]
pub struct FaultController(Arc<Mutex<FaultState>>);

#[cfg_attr(
    any(
        not(test),
        not(any(target_vendor = "apple", target_os = "linux", target_os = "android"))
    ),
    allow(dead_code)
)]
impl FaultController {
    /// Make a controller with the specified rules.
    #[must_use]
    pub fn new(rules: impl IntoIterator<Item = FaultRule>) -> Self {
        let rules = rules
            .into_iter()
            .map(|rule| RuleState {
                rule,
                consumed: false,
            })
            .collect();
        Self(Arc::new(Mutex::new(FaultState {
            observed: BTreeMap::new(),
            rules,
            #[cfg(test)]
            test_hook: TestHook::default(),
        })))
    }

    #[cfg(test)]
    pub(crate) fn set_test_hook(&self, hook: impl FnOnce() + Send + 'static) -> StorageResult<()> {
        let mut state = self.lock(&StorageContext::root_open())?;
        state.test_hook = TestHook(Some(Box::new(hook)));
        Ok(())
    }

    /// Get the number of rules that did not fire.
    pub fn remaining_rules(&self) -> StorageResult<usize> {
        let state = self.lock(&StorageContext::root_open())?;
        Ok(state.rules.iter().filter(|rule| !rule.consumed).count())
    }

    /// Report whether all rules fired.
    pub fn is_exhausted(&self) -> StorageResult<bool> {
        Ok(self.remaining_rules()? == 0)
    }

    pub(crate) fn before(&self, context: &StorageContext) -> StorageResult<()> {
        let mut state = self.lock(context)?;
        let key = (context.operation, context.step);
        let occurrence = state.observed.entry(key).or_insert(0);
        *occurrence = occurrence.wrapping_add(1);
        let occurrence = *occurrence;
        Self::fire(&mut state, context, occurrence, FaultAction::FailBefore)
    }

    pub(crate) fn after(&self, context: &StorageContext) -> StorageResult<()> {
        let mut state = self.lock(context)?;
        let occurrence = state
            .observed
            .get(&(context.operation, context.step))
            .copied()
            .unwrap_or(0);
        Self::fire(&mut state, context, occurrence, FaultAction::LoseAckAfter)
    }

    fn fire(
        state: &mut FaultState,
        context: &StorageContext,
        occurrence: u64,
        action: FaultAction,
    ) -> StorageResult<()> {
        if let Some(rule) = state.rules.iter_mut().find(|candidate| {
            !candidate.consumed
                && candidate.rule.operation == context.operation
                && candidate.rule.step == context.step
                && candidate.rule.occurrence == occurrence
                && candidate.rule.action == action
        }) {
            rule.consumed = true;
            #[cfg(test)]
            if let Some(hook) = state.test_hook.0.take() {
                hook();
            }
            return Err(StorageError::InjectedFault {
                context: context.clone(),
            });
        }
        Ok(())
    }

    fn lock<'a>(
        &'a self,
        context: &StorageContext,
    ) -> StorageResult<std::sync::MutexGuard<'a, FaultState>> {
        self.0
            .lock()
            .map_err(|_| StorageError::FaultControllerPoisoned {
                context: context.clone(),
            })
    }
}
